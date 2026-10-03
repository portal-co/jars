#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/jars-agent-common.sh"

PI_MODEL='openai-codex/gpt-6-luna:xhigh' PI_PROVIDER=ignored jars_build_pi_args
[[ "${JARS_PI_ARGS[*]}" == '--model openai-codex/gpt-6-luna:xhigh' ]]

PI_MODEL=gpt-5 PI_PROVIDER=test-provider PI_THINKING=high jars_build_pi_args
[[ "${JARS_PI_ARGS[*]}" == '--provider test-provider --model gpt-5 --thinking high' ]]

PI_MODEL=gpt-5 PI_PROVIDER=test-provider PI_THINKING_LEVEL=xhigh jars_build_pi_args
[[ "${JARS_PI_ARGS[*]}" == '--provider test-provider --model gpt-5 --thinking xhigh' ]]

if PI_MODEL=gpt-5 PI_THINKING=invalid jars_build_pi_args 2>/dev/null; then
    echo 'invalid thinking level was accepted' >&2
    exit 1
fi
if PI_MODEL='gpt-5:low' PI_THINKING=high jars_build_pi_args 2>/dev/null; then
    echo 'conflicting thinking levels were accepted' >&2
    exit 1
fi

jars_setup_java >/dev/null
[[ -x "$JAVAC" ]]
[[ -x "$JAVA_HOME/bin/javac" ]]

echo 'shared Pi model/thinking and Java discovery tests passed'
