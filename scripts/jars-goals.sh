#!/usr/bin/env bash
# Run independent goal files in parallel Pi sessions and isolated git worktrees.
# Successful, non-conflicting branches are merged into the current checkout;
# remaining committed work can be pushed and opened as GitHub pull requests.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(git -C "$PWD" rev-parse --show-toplevel 2>/dev/null || git -C "$SCRIPT_DIR/.." rev-parse --show-toplevel)"
# shellcheck source=scripts/jars-agent-common.sh
source "$SCRIPT_DIR/jars-agent-common.sh"

WORKERS="${JARS_GOALS_WORKERS:-3}"
TIMEOUT_SECONDS="${TIMEOUT_SECONDS:-5400}"
REMOTE="${JARS_GOALS_REMOTE:-origin}"
BASE_REF="HEAD"
PR_BASE=""
RUN_ID="$(date '+%Y%m%d%H%M%S')-$$"
RUN_DIR=""
NO_MERGE=0
NO_PR=0
KEEP_WORKTREES=0
DRY_RUN=0
GOALS=()
SLUGS=()
STATUSES=()
BRANCHES=()
WORKTREES=()

usage() {
    cat <<'USAGE'
Usage: scripts/jars-goals.sh [options] GOAL_FILE...

Run one Pi task per goal file, concurrently in separate git worktrees.
Successful branches are merged in order when possible. Non-mergeable or
unsuccessful branches are pushed and opened as PRs unless --no-pr is set.

Options:
  --repo DIR           Repository to work in (default: current git repository)
  --workers N          Maximum concurrent Pi sessions (default: 3)
  --base REF           Base commit/ref for every worktree (default: HEAD)
  --pr-base BRANCH     Target branch for PRs (default: current branch)
  --remote NAME        Git remote to push to (default: origin)
  --run-id ID          Stable identifier used for branches and run directory
  --run-dir DIR        Directory for logs, prompts, results and worktrees
  --timeout SECONDS    Per-goal Pi timeout (default: 5400)
  --no-merge           Do not merge results into the current checkout
  --no-pr              Do not push branches or create PRs
  --keep-worktrees     Keep goal worktrees after processing
  --dry-run            Print the plan without creating worktrees or invoking Pi
  -h, --help           Show this help

Model configuration is shared with jars-ralph.sh:
  PI_MODEL=provider/model:thinking (or a plain model name)
  PI_PROVIDER=provider       Used when PI_MODEL has no provider prefix
  PI_THINKING=level          off|minimal|low|medium|high|xhigh|max
  JAVA_HOME/JDK_HOME/OPENJDK_HOME/JAVAC select a JDK; otherwise Java 25 is autodiscovered first
USAGE
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --repo) REPO="$2"; shift 2 ;;
        --workers) WORKERS="$2"; shift 2 ;;
        --base) BASE_REF="$2"; shift 2 ;;
        --pr-base) PR_BASE="$2"; shift 2 ;;
        --remote) REMOTE="$2"; shift 2 ;;
        --run-id) RUN_ID="$2"; shift 2 ;;
        --run-dir) RUN_DIR="$2"; shift 2 ;;
        --timeout) TIMEOUT_SECONDS="$2"; shift 2 ;;
        --no-merge) NO_MERGE=1; shift ;;
        --no-pr) NO_PR=1; shift ;;
        --keep-worktrees) KEEP_WORKTREES=1; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        -h|--help) usage; exit 0 ;;
        --) shift; while [[ $# -gt 0 ]]; do GOALS+=("$1"); shift; done ;;
        -*) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
        *) GOALS+=("$1"); shift ;;
    esac
done

if [[ ${#GOALS[@]} -eq 0 ]]; then
    echo "provide at least one goal file" >&2
    usage >&2
    exit 2
fi
if [[ ! "$WORKERS" =~ ^[1-9][0-9]*$ ]]; then
    echo "--workers must be a positive integer" >&2
    exit 2
fi
if [[ ! "$TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]]; then
    echo "--timeout must be a positive integer" >&2
    exit 2
fi
if [[ ! "$RUN_ID" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "--run-id may contain only letters, digits, dot, underscore, and hyphen" >&2
    exit 2
fi

REPO="$(cd "$REPO" && pwd)"
BASE_COMMIT="$(git -C "$REPO" rev-parse --verify "$BASE_REF^{commit}")"
BASE_BRANCH="$(git -C "$REPO" symbolic-ref --quiet --short HEAD || true)"
PR_BASE="${PR_BASE:-$BASE_BRANCH}"
if [[ -z "$RUN_DIR" ]]; then
    RUN_DIR="$REPO/../jars-goal-runs/$RUN_ID"
fi
RUN_DIR="$(mkdir -p "$RUN_DIR" && cd "$RUN_DIR" && pwd)"
WORKTREE_ROOT="$RUN_DIR/worktrees"

for i in "${!GOALS[@]}"; do
    goal="${GOALS[$i]}"
    if [[ ! -f "$goal" || ! -r "$goal" ]]; then
        echo "goal is not a readable file: $goal" >&2
        exit 2
    fi
    goal="$(cd "$(dirname "$goal")" && pwd)/$(basename "$goal")"
    GOALS[$i]="$goal"
    name="$(basename "$goal")"
    name="${name%.*}"
    slug="$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]' | sed -E 's/[^a-z0-9]+/-/g; s/^-|-$//g' | cut -c1-48)"
    [[ -n "$slug" ]] || slug="goal"
    SLUGS[$i]="$(printf '%03d-%s' "$((i + 1))" "$slug")"
    BRANCHES[$i]="jars-goals/$RUN_ID/${SLUGS[$i]}"
    WORKTREES[$i]="$WORKTREE_ROOT/${SLUGS[$i]}"
done

jars_build_pi_args
if [[ "$DRY_RUN" != 1 && "$NO_MERGE" != 1 ]] && [[ -n "$(git -C "$REPO" status --porcelain)" ]]; then
    echo "refusing to merge into a dirty checkout; commit/stash changes or use --no-merge" >&2
    exit 2
fi
if [[ "$NO_PR" != 1 && "$DRY_RUN" != 1 && -z "$PR_BASE" ]]; then
    echo "cannot open PRs from a detached HEAD; pass --pr-base explicitly" >&2
    exit 2
fi

if [[ "$DRY_RUN" == 1 ]]; then
    printf 'repo: %s\nbase: %s\nmodel args:' "$REPO" "$BASE_COMMIT"
    printf ' %q' "${JARS_PI_ARGS[@]}"
    printf '\nworkers: %s\n' "$WORKERS"
    for i in "${!GOALS[@]}"; do
        printf '  %s -> %s (%s)\n' "${GOALS[$i]}" "${BRANCHES[$i]}" "${WORKTREES[$i]}"
    done
    exit 0
fi

jars_setup_java
mkdir -p "$RUN_DIR/logs" "$RUN_DIR/prompts" "$RUN_DIR/results" "$WORKTREE_ROOT"
printf 'base_commit=%s\nbase_branch=%s\nmodel=%s\nthinking=%s\n' \
    "$BASE_COMMIT" "$BASE_BRANCH" "${PI_MODEL:-glm-5.3-flash}" \
    "${PI_THINKING:-${PI_THINKING_LEVEL:-embedded-or-default}}" >"$RUN_DIR/run.txt"

build_goal_prompt() {
    local goal="$1"
    {
        cat <<'PROMPT'
Implement the following repository goal. First read the repository's agent instructions and inspect the relevant code. Treat the goal file as the specification: complete a coherent implementation, add or update tests, run relevant verification, and commit the finished changes. If blocked, leave a clear explanation in the final response and do not claim completion.

Goal file contents:
---
PROMPT
        cat "$goal"
        printf '\n---\n'
        jars_agent_shared_instructions
    }
}

run_goal() {
    local index="$1"
    local goal="${GOALS[$index]}"
    local slug="${SLUGS[$index]}"
    local branch="${BRANCHES[$index]}"
    local worktree="${WORKTREES[$index]}"
    local log="$RUN_DIR/logs/$slug.log"
    local prompt_file="$RUN_DIR/prompts/$slug.md"
    local result="$RUN_DIR/results/$slug.status"
    local pi_status=0
    local status

    build_goal_prompt "$goal" >"$prompt_file"
    if ! git -C "$REPO" worktree add -b "$branch" "$worktree" "$BASE_COMMIT" >"$RUN_DIR/logs/$slug.worktree.log" 2>&1; then
        echo setup-failed >"$result"
        return 1
    fi

    echo "[goals] starting $slug in $worktree"
    if (cd "$worktree" && jars_run_pi "$log" "$TIMEOUT_SECONDS" "$(<"$prompt_file")"); then
        pi_status=0
    else
        pi_status=$?
        echo "[goals] $slug Pi exited with status $pi_status" | tee -a "$log"
    fi

    # Some Pi sessions leave verified changes unstaged. Preserve them as a
    # normal branch commit so merging/review remains deterministic.
    if [[ -n "$(git -C "$worktree" status --porcelain --untracked-files=all)" ]]; then
        if git -C "$worktree" add -A && \
            git -C "$worktree" -c user.name='Pi Agent' -c user.email='pi-agent@users.noreply.github.com' \
                commit -m "[AI] Complete goal: ${slug#*-}" >>"$log" 2>&1; then
            :
        else
            echo "[goals] could not commit remaining changes for $slug" | tee -a "$log"
        fi
    fi

    if [[ "$(git -C "$worktree" rev-parse HEAD)" == "$BASE_COMMIT" ]]; then
        status="no-change"
    elif [[ "$pi_status" -eq 0 ]] && [[ -z "$(git -C "$worktree" status --porcelain --untracked-files=all)" ]]; then
        status="success"
    else
        status="agent-failed-$pi_status"
    fi
    echo "$status" >"$result"
    tail -25 "$log" || true
    echo "[goals] $slug: $status"
    return 0
}

# Run bounded batches rather than relying on GNU parallel or Bash wait -n,
# neither of which is guaranteed on the macOS host.
for ((start = 0; start < ${#GOALS[@]}; start += WORKERS)); do
    pids=()
    end=$((start + WORKERS))
    (( end > ${#GOALS[@]} )) && end=${#GOALS[@]}
    for ((i = start; i < end; i++)); do
        run_goal "$i" &
        pids+=("$!")
    done
    batch_failed=0
    for pid in "${pids[@]}"; do
        if ! wait "$pid"; then batch_failed=1; fi
    done
    if [[ "$batch_failed" == 1 ]]; then
        echo "[goals] at least one worker failed to initialize; see $RUN_DIR/logs" >&2
    fi
done

merge_or_publish() {
    local i status branch worktree has_commit commit_count
    local body
    for i in "${!GOALS[@]}"; do
        status="$(<"$RUN_DIR/results/${SLUGS[$i]}.status")"
        branch="${BRANCHES[$i]}"
        worktree="${WORKTREES[$i]}"
        has_commit=0
        commit_count=0
        if git -C "$REPO" show-ref --verify --quiet "refs/heads/$branch"; then
            commit_count="$(git -C "$REPO" rev-list --count "$BASE_COMMIT..$branch" 2>/dev/null || echo 0)"
            if [[ "$commit_count" =~ ^[0-9]+$ ]] && [[ "$commit_count" -gt 0 ]]; then
                has_commit=1
            fi
        fi

        if [[ "$has_commit" == 1 && "$NO_MERGE" != 1 && "$status" == success ]]; then
            if git -C "$REPO" merge --no-edit "$branch"; then
                status=merged
                echo "merged" >"$RUN_DIR/results/${SLUGS[$i]}.status"
            else
                git -C "$REPO" merge --abort >/dev/null 2>&1 || true
                status=conflict
                echo "conflict" >"$RUN_DIR/results/${SLUGS[$i]}.status"
                echo "[goals] merge conflict for $branch; will offer it as a PR"
            fi
        fi

        if [[ "$has_commit" == 1 && "$status" != merged && "$NO_PR" != 1 ]]; then
            if ! command -v gh >/dev/null 2>&1 || ! git -C "$REPO" remote get-url "$REMOTE" >/dev/null 2>&1; then
                echo "[goals] $branch needs a PR, but gh or remote '$REMOTE' is unavailable"
            elif [[ -z "$PR_BASE" ]]; then
                echo "[goals] $branch needs a PR; use --pr-base because HEAD is detached"
            else
                body="$(printf 'Automated implementation for goal file: %s\n\nPi model: %s\nRun: %s' \
                    "${GOALS[$i]}" "${PI_MODEL:-glm-5.3-flash}" "$RUN_ID")"
                if git -C "$REPO" push -u "$REMOTE" "$branch" && \
                    (cd "$REPO" && gh pr create --head "$branch" --base "$PR_BASE" \
                        --title "[AI] Goal: ${SLUGS[$i]}" --body "$body"); then
                    echo "[goals] opened PR for $branch"
                else
                    echo "[goals] could not publish $branch; branch remains available locally" >&2
                fi
            fi
        elif [[ "$has_commit" == 1 && "$NO_PR" == 1 && "$status" != merged ]]; then
            echo "[goals] preserved branch $branch (PR creation disabled)"
        elif [[ "$has_commit" == 0 ]]; then
            echo "[goals] $branch produced no commits"
        fi

        if [[ "$KEEP_WORKTREES" != 1 && -e "$worktree" ]]; then
            git -C "$REPO" worktree remove --force "$worktree" >/dev/null 2>&1 || \
                echo "[goals] could not remove worktree $worktree" >&2
        fi
    done
}

if [[ "$NO_MERGE" != 1 ]] && [[ -n "$(git -C "$REPO" status --porcelain)" ]]; then
    echo "[goals] checkout became dirty while agents were running; skipping merges" >&2
    NO_MERGE=1
fi
merge_or_publish

echo "[goals] results: $RUN_DIR/results"
printf '[goals] summary: '
for i in "${!GOALS[@]}"; do
    printf '%s=%s ' "${SLUGS[$i]}" "$(<"$RUN_DIR/results/${SLUGS[$i]}.status")"
done
printf '\n'
