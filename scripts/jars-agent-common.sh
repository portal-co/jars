#!/usr/bin/env bash
# Shared Pi invocation and execution contract for the jars agent loops.

jars_agent_shared_instructions() {
    cat <<'INSTRUCTIONS'

## Shared execution contract

- Follow the repository's AGENTS.md/CLAUDE.md instructions and existing code style.
- Make the smallest complete change for the assigned task; add or update tests.
- Use the runner-provided `JAVA_HOME`/`JAVAC` for Java fixtures rather than hardcoding a machine-specific compiler path.
- Treat build and test failures as problems to diagnose and fix, not as a reason to revert sound work or stop. Inspect the failure, fix the root cause, rerun the failed check, then run broader relevant verification.
- Do not discard task changes merely to get a green run. Revert only changes shown to be incorrect, and preserve unrelated work.
- Keep iterating until relevant checks pass or a concrete external blocker remains; report any remaining failures accurately.
- Commit completed work with a descriptive `[AI]`-prefixed message.
INSTRUCTIONS
}

jars_java_major() {
    local compiler="$1"
    local version
    version="$("$compiler" -version 2>&1 || true)"
    printf '%s\n' "$version" | sed -nE 's/^javac ([0-9]+).*$/\1/p'
}

jars_setup_java() {
    local java_home=""
    local javac=""
    local candidate=""
    local version=""

    # Explicit compiler/home settings win; tolerate JAVAC being either a path
    # or a command name supplied by the caller.
    if [[ -n "${JAVAC:-}" ]]; then
        if [[ -x "$JAVAC" ]]; then
            javac="$JAVAC"
        elif command -v "$JAVAC" >/dev/null 2>&1; then
            javac="$(command -v "$JAVAC")"
        fi
    fi
    for candidate in "${JAVA_HOME:-}" "${JDK_HOME:-}" "${OPENJDK_HOME:-}"; do
        if [[ -n "$candidate" && -x "$candidate/bin/javac" ]]; then
            java_home="$candidate"
            if [[ -z "$javac" ]]; then
                javac="$candidate/bin/javac"
            fi
            break
        fi
    done

    # java_home can return a lower version even for `-v 25` on macOS, so
    # verify its javac version instead of trusting the returned path.
    if [[ -z "$javac" ]] && [[ -x /usr/libexec/java_home ]]; then
        candidate="$(/usr/libexec/java_home -v 25 2>/dev/null || true)"
        if [[ -n "$candidate" && -x "$candidate/bin/javac" ]] && \
            [[ "$(jars_java_major "$candidate/bin/javac")" == 25 ]]; then
            java_home="$candidate"
            javac="$candidate/bin/javac"
        fi
    fi
    if [[ -z "$javac" ]]; then
        for candidate in \
            /opt/homebrew/opt/openjdk@25 \
            /opt/homebrew/opt/openjdk@25/libexec/openjdk.jdk/Contents/Home \
            /usr/local/opt/openjdk@25 \
            /usr/local/opt/openjdk@25/libexec/openjdk.jdk/Contents/Home \
            /opt/homebrew/opt/openjdk@21 \
            /opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
            /opt/homebrew/opt/openjdk \
            /opt/homebrew/opt/openjdk/libexec/openjdk.jdk/Contents/Home \
            /usr/local/opt/openjdk@21 \
            /usr/local/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home \
            /usr/local/opt/openjdk \
            /usr/local/opt/openjdk/libexec/openjdk.jdk/Contents/Home \
            /usr/lib/jvm/java-21-openjdk-amd64 \
            /usr/lib/jvm/java-21-openjdk \
            /usr/lib/jvm/default-java \
            /Library/Java/JavaVirtualMachines/*/Contents/Home; do
            if [[ -x "$candidate/bin/javac" ]]; then
                java_home="$candidate"
                javac="$candidate/bin/javac"
                break
            fi
        done
    fi

    # Finally use javac from PATH (including platform shims such as macOS's
    # /usr/bin/javac) if no JDK home was discoverable.
    if [[ -z "$javac" ]] && command -v javac >/dev/null 2>&1; then
        javac="$(command -v javac)"
    fi
    if [[ -z "$java_home" && -x /usr/libexec/java_home && -n "$javac" ]]; then
        local major
        major="$(jars_java_major "$javac")"
        if [[ -n "$major" ]]; then
            candidate="$(/usr/libexec/java_home -v "$major" 2>/dev/null || true)"
            if [[ -n "$candidate" && -x "$candidate/bin/javac" ]] && \
                [[ "$(jars_java_major "$candidate/bin/javac")" == "$major" ]]; then
                java_home="$candidate"
            fi
        fi
    fi
    if [[ -z "$java_home" && -n "$javac" ]]; then
        candidate="$(dirname "$(dirname "$javac")")"
        if [[ "$candidate" != "/" && "$candidate" != "/usr" ]] && \
            [[ -x "$candidate/bin/javac" ]]; then
            java_home="$candidate"
        fi
    fi

    if [[ -n "$java_home" ]]; then
        export JAVA_HOME="$java_home"
        PATH="$JAVA_HOME/bin:$PATH"
        export PATH
    elif [[ -n "$javac" ]]; then
        PATH="$(dirname "$javac"):$PATH"
        export PATH
    fi
    if [[ -n "$javac" ]]; then
        export JAVAC="$javac"
        export JARS_JAVAC="$javac"
        version="$("$javac" -version 2>&1 || true)"
        echo "[agent] Java compiler: $javac ($version)"
    else
        echo "[agent] warning: OpenJDK not found; set JAVA_HOME, JDK_HOME, OPENJDK_HOME, or JAVAC" >&2
    fi
}

jars_build_pi_args() {
    local model="${PI_MODEL:-glm-5.3-flash}"
    local provider="${PI_PROVIDER:-surplus-intelligence}"
    local thinking="${PI_THINKING:-${PI_THINKING_LEVEL:-}}"
    local suffix=""
    local valid_thinking=" off minimal low medium high xhigh max "

    if [[ "$model" == */* ]]; then
        JARS_PI_ARGS=(--model "$model")
    else
        JARS_PI_ARGS=(--provider "$provider" --model "$model")
    fi

    # Pi accepts model:thinking shorthand. Preserve it as one model pattern,
    # while also accepting PI_THINKING for callers that keep the settings apart.
    if [[ "$model" == *:* ]]; then
        suffix="${model##*:}"
        if [[ "$valid_thinking" != *" $suffix "* ]]; then
            echo "invalid thinking level in PI_MODEL '$model'" >&2
            return 2
        fi
        if [[ -n "$thinking" && "$thinking" != "$suffix" ]]; then
            echo "PI_THINKING ('$thinking') conflicts with PI_MODEL suffix '$suffix'" >&2
            return 2
        fi
    elif [[ -n "$thinking" ]]; then
        if [[ "$valid_thinking" != *" $thinking "* ]]; then
            echo "invalid PI_THINKING '$thinking' (expected off|minimal|low|medium|high|xhigh|max)" >&2
            return 2
        fi
        JARS_PI_ARGS+=(--thinking "$thinking")
    fi
}

jars_run_pi() {
    local log="$1"
    local timeout_seconds="$2"
    local prompt="$3"
    local pi_bin="${PI_BIN:-pi}"

    jars_build_pi_args || return $?
    if command -v timeout >/dev/null 2>&1; then
        timeout "$timeout_seconds" "$pi_bin" "${JARS_PI_ARGS[@]}" \
            --mode text --no-session -p "$prompt" >"$log" 2>&1
    else
        # macOS ships without GNU timeout. Perl's alarm bounds the child process.
        perl -e 'alarm shift; exec @ARGV' "$timeout_seconds" "$pi_bin" \
            "${JARS_PI_ARGS[@]}" --mode text --no-session -p "$prompt" >"$log" 2>&1
    fi
}
