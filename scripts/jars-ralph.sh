#!/usr/bin/env bash
# jars-ralph.sh — agentic stdlib-expansion loop for the jars compiler.
#
# Each iteration runs a fresh, isolated Pi session inside a dedicated git
# worktree. The agent reads the whole-JAR coverage report, picks the highest-
# leverage unmodeled platform classes, expands the stdlib declaration one
# slice, commits, and exits. The loop verifies progress via the coverage
# ratchet: if ok_methods does not increase, the iteration is a miss; after
# MAX_MISSES consecutive misses the loop stops.
#
# Usage:
#   scripts/jars-ralph.sh                       # loop until goal or misses
#   scripts/jars-ralph.sh --iterations 5        # cap the number of iterations
#   scripts/jars-ralph.sh --dry-run             # print the first prompt, exit
#   scripts/jars-ralph.sh --resume              # reuse the existing worktree
#
# Environment:
#   PI_MODEL          model pattern passed to pi (default: google/gemini-2.5-pro)
#   MAX_MISSES        consecutive non-improving iterations before stopping (default: 3)
#   GOAL_OK_METHODS   stop when ok_methods reaches this count (default: 0 = no numeric goal)
#   TIMEOUT_SECONDS   per-iteration pi timeout (default: 5400)

set -euo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKTREE="${JARS_RALPH_WORKTREE:-$REPO/../jars-ralph-worktree}"
BRANCH="jars-ralph"
MAX_MISSES="${MAX_MISSES:-3}"
GOAL_OK_METHODS="${GOAL_OK_METHODS:-0}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-5400}"
ITERATIONS=0
LIMIT=0
DRY_RUN=0
RESUME=0

while [[ $# -gt 0 ]]; do
    case "$1" in
        --iterations) LIMIT="$2"; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        --resume) RESUME=1; shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

snapshot_value() {
    # snapshot_value <key> — read a metric from the ratchet snapshot
    awk -v key="$1" '$1 == key { print $2 }' \
        "$REPO/crates/jars-core/tests/coverage-snapshot/commons-lang3-3.14.0.txt" 2>/dev/null || echo 0
}

git_in_worktree() {
    git -C "$WORKTREE" "$@"
}

setup_worktree() {
    if [[ -d "$WORKTREE/.git" && "$RESUME" == "1" ]]; then
        echo "[ralph] resuming existing worktree at $WORKTREE"
        git_in_worktree rebase "$REPO" 2>/dev/null || git_in_worktree merge -X theirs "$REPO" || true
        return
    fi
    if git -C "$REPO" worktree list | grep -q "$WORKTREE"; then
        echo "[ralph] worktree already registered at $WORKTREE"
    else
        git -C "$REPO" worktree add "$WORKTREE" -b "$BRANCH" 2>/dev/null \
            || git -C "$REPO" worktree add "$WORKTREE" "$BRANCH"
    fi
    echo "[ralph] worktree ready at $WORKTREE (branch $BRANCH)"
}

build_prompt() {
    local ok_methods
    ok_methods="$(snapshot_value ok_methods)"
    cat <<PROMPT
You are working in the jars repository: a Java-to-Rust AOT compiler that
compiles a closed set of Java classes ahead of time into actor-model Rust.
Your single task this session: expand the modeled JDK surface so that more of
the whole-JAR compile coverage ratchet passes, then commit.

## Where things stand

The ratchet snapshot (crates/jars-core/tests/coverage-snapshot/) currently
records ok_methods = ${ok_methods} across the vendored commons-lang3-3.14.0.jar.
The goal of this loop is full commons-lang coverage: every public static
method of org/apache/commons/lang3/StringUtils (and the classes it needs)
compiles.

## How to make progress (one slice per session)

1. Run: cargo run -q -p jars-core --bin jars-coverage -- 21 crates/jars-core/tests/jar-fixtures/third-party/commons-lang3-3.14.0.jar
   Study the report. Unmodeled platform classes and per-class errors tell you
   exactly which JDK members are missing and which opcodes fail.
2. Pick a small, coherent slice of missing JDK surface (one to five related
   members or one opcode family) that unblocks coverage. Prefer slices that
   unblock StringUtils methods.
3. Declare the new members in crates/jars-stdlib/src/lib.rs — the single
   shared declaration that generates both the runtime implementations and the
   compiler's member table. Follow the existing conventions exactly:
   stdlib lowerings produce the full frame value themselves (Option<...> for
   reference returns, bool for booleans); the actor model is non-negotiable
   (mailbox messages for state, no shared mutable state, no locks held
   across await).
4. Extend the typed-frame lowering in crates/jars-core/src/lib.rs if the
   slice needs a bytecode opcode that is not yet supported. Emit more Rust;
   never add a bytecode interpreter.
5. Add or extend a split fixture under crates/jars-core/tests/jar-fixtures/
   if the slice adds user-visible StringUtils behavior (see commons-lang-*.toml
   files; ground-truth expected_stdout comes from running the app on the real
   JVM with the vendored JAR).
6. Verify: cargo test --workspace && cargo clippy -p jars-core -p jars-runtime -p jars-stdlib --all-targets
   All tests must pass; no new clippy warnings.
7. Refresh the ratchet only if coverage improved:
   UPDATE_COVERAGE_SNAPSHOT=1 cargo test -p jars-core --test coverage_ratchet
8. Commit with a descriptive message naming the slice. Do not push.

## Hard rules (from AGENTS.md)

- No class files, bytecode interpretation, dynamic loading, or reflection in
  the generated program.
- No holding the actor state mutex across an await.
- No shared mutable object state outside an actor.
- Overloads collide on generated fn names: exercise at most one overload per
  method name per fixture app.

Begin. Be surgical: one slice, verified, committed. If you cannot find a
slice that increases ok_methods this session, say so and stop without
committing a snapshot regression.
PROMPT
}

run_iteration() {
    local prompt
    prompt="$(build_prompt)"
    if [[ "$DRY_RUN" == "1" ]]; then
        echo "$prompt"
        return 0
    fi
    echo "[ralph] iteration $ITERATIONS: launching pi in $WORKTREE"
    local log
    log="$(mktemp -t jars-ralph.XXXXXX)"
    if command -v timeout >/dev/null 2>&1; then
        timeout "$TIMEOUT_SECONDS" \
            git -C "$WORKTREE" rev-parse --verify HEAD >/dev/null 2>&1 && \
            (cd "$WORKTREE" && timeout "$TIMEOUT_SECONDS" pi \
                --provider "${PI_PROVIDER:-google}" \
                --model "${PI_MODEL:-gemini-2.5-pro}" \
                --mode text \
                --no-session \
                -p "$prompt") >"$log" 2>&1 || true
    else
        # macOS: no GNU timeout by default; fall back to perl alarm.
        (cd "$WORKTREE" && perl -e 'alarm shift; exec @ARGV' "$TIMEOUT_SECONDS" \
            pi --provider "${PI_PROVIDER:-google}" \
            --model "${PI_MODEL:-gemini-2.5-pro}" \
            --mode text \
            --no-session \
            -p "$prompt") >"$log" 2>&1 || true
    fi
    tail -40 "$log"
    echo "[ralph] iteration log: $log"
}

main() {
    setup_worktree
    local misses=0
    local previous
    previous="$(snapshot_value ok_methods)"
    while :; do
        ITERATIONS=$((ITERATIONS + 1))
        if [[ "$LIMIT" != "0" && "$ITERATIONS" -gt "$LIMIT" ]]; then
            echo "[ralph] iteration limit reached"
            break
        fi
        run_iteration
        if [[ "$DRY_RUN" == "1" ]]; then
            return 0
        fi
        # The agent commits in the worktree; merge advancing commits back so
        # the main checkout's snapshot reflects progress.
        if git_in_worktree rev-parse --verify HEAD >/dev/null 2>&1; then
            git -C "$REPO" merge -X theirs --no-edit "$BRANCH" 2>/dev/null \
                || echo "[ralph] warning: merge of $BRANCH into main failed; investigate manually"
        fi
        local current
        current="$(snapshot_value ok_methods)"
        echo "[ralph] ok_methods: $previous -> $current"
        if [[ "$GOAL_OK_METHODS" != "0" && "$current" -ge "$GOAL_OK_METHODS" ]]; then
            echo "[ralph] goal reached: ok_methods = $current"
            break
        fi
        if [[ "$current" -le "$previous" ]]; then
            misses=$((misses + 1))
            echo "[ralph] no progress ($misses/$MAX_MISSES misses)"
            if [[ "$misses" -ge "$MAX_MISSES" ]]; then
                echo "[ralph] stopping after $MAX_MISSES consecutive misses"
                break
            fi
        else
            misses=0
        fi
        previous="$current"
    done
    echo "[ralph] done after $ITERATIONS iterations"
}

main "$@"
