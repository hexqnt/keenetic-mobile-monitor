#!/usr/bin/env bash

set -Eeuo pipefail

readonly SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
readonly BINARY_NAME="keenetic-mobile-monitor"
readonly PROFILE="profiling"

usage() {
    cat <<EOF
Usage: $(basename -- "$0") [--delay SECONDS] [--] <application arguments...>

Builds $BINARY_NAME with the Cargo '$PROFILE' profile, runs it under perf,
and stores the captured data, executable, metadata, CPU report, and resource
usage together.

Example:
  ./profiling.sh --delay 60 -- serve --config config.toml

Options:
  -d, --delay SECONDS   Warm-up time before perf starts sampling (default: 0)
  -h, --help            Show this help

Environment variables:
  PROFILING_OUTPUT_DIR  Results directory (default: ./profiling-results)
  PERF_EVENT            Sampling event (default: cycles:u)
  PERF_FREQUENCY        Sampling frequency in Hz (default: 521)
  PERF_CALL_GRAPH       Call-graph method (default: dwarf,16384)
EOF
}

delay_seconds=0
while (($# > 0)); do
    case "$1" in
        -d | --delay)
            if (($# < 2)); then
                printf 'Error: %s requires a value in seconds.\n' "$1" >&2
                exit 2
            fi
            delay_seconds="$2"
            shift 2
            ;;
        --delay=*)
            delay_seconds="${1#*=}"
            shift
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        *)
            break
            ;;
    esac
done

if (($# == 0)); then
    usage >&2
    exit 2
fi

for command in cargo perf; do
    if ! command -v "$command" >/dev/null 2>&1; then
        printf 'Error: required command not found: %s\n' "$command" >&2
        exit 127
    fi
done

readonly RESULTS_ROOT="${PROFILING_OUTPUT_DIR:-$SCRIPT_DIR/profiling-results}"
readonly DELAY_SECONDS="$delay_seconds"
readonly PERF_EVENT="${PERF_EVENT:-cycles:u}"
readonly PERF_FREQUENCY="${PERF_FREQUENCY:-521}"
readonly PERF_CALL_GRAPH="${PERF_CALL_GRAPH:-dwarf,16384}"

if [[ ! "$DELAY_SECONDS" =~ ^[0-9]+$ ]]; then
    printf 'Error: --delay must be a non-negative integer.\n' >&2
    exit 2
fi

readonly PERF_DELAY_MILLISECONDS=$((10#$DELAY_SECONDS * 1000))

cd -- "$SCRIPT_DIR"
printf 'Building %s with the Cargo %s profile...\n' "$BINARY_NAME" "$PROFILE"
cargo build --profile "$PROFILE" --bin "$BINARY_NAME"

readonly TARGET_DIR="${CARGO_TARGET_DIR:-$SCRIPT_DIR/target}"
readonly BUILT_BINARY="$TARGET_DIR/$PROFILE/$BINARY_NAME"
if [[ ! -x "$BUILT_BINARY" ]]; then
    printf 'Error: built executable not found: %s\n' "$BUILT_BINARY" >&2
    exit 1
fi

mkdir -p -- "$RESULTS_ROOT"
readonly RUN_DIR="$(mktemp -d -- "$RESULTS_ROOT/$(date +%Y%m%d-%H%M%S)-XXXXXX")"
readonly SAVED_BINARY="$RUN_DIR/$BINARY_NAME"
readonly PERF_DATA="$RUN_DIR/perf.data"
readonly PERF_REPORT="$RUN_DIR/perf-report.txt"
readonly RESOURCE_USAGE="$RUN_DIR/resource-usage.txt"
readonly METADATA="$RUN_DIR/metadata.txt"

cp --preserve=mode,timestamps -- "$BUILT_BINARY" "$SAVED_BINARY"

{
    printf 'created_at=%s\n' "$(date --iso-8601=seconds)"
    printf 'git_commit=%s\n' "$(git rev-parse --verify HEAD 2>/dev/null || printf unknown)"
    printf 'rustc=%s\n' "$(rustc --version)"
    printf 'perf=%s\n' "$(perf version)"
    printf 'event=%s\n' "$PERF_EVENT"
    printf 'delay_seconds=%s\n' "$DELAY_SECONDS"
    printf 'resource_usage_scope=entire_application_run\n'
    printf 'frequency_hz=%s\n' "$PERF_FREQUENCY"
    printf 'call_graph=%s\n' "$PERF_CALL_GRAPH"
    printf 'command='
    printf '%q ' "$SAVED_BINARY" "$@"
    printf '\n'
} >"$METADATA"

declare -a perf_delay_args=()
if ((PERF_DELAY_MILLISECONDS > 0)); then
    perf_delay_args=(--delay "$PERF_DELAY_MILLISECONDS")
    printf 'Starting application; perf sampling will begin after %s seconds...\n' "$DELAY_SECONDS"
else
    printf 'Recording profile. Stop the application normally or press Ctrl+C...\n'
fi

declare -a application_command=("$SAVED_BINARY" "$@")
if [[ -x /usr/bin/time ]] && /usr/bin/time --version 2>&1 | head -n 1 | grep -q 'GNU time'; then
    application_command=(/usr/bin/time --verbose --output "$RESOURCE_USAGE" -- "$SAVED_BINARY" "$@")
else
    printf 'Warning: GNU time not found; resource usage will not be recorded.\n' >&2
fi

set +e
perf record \
    --output "$PERF_DATA" \
    --event "$PERF_EVENT" \
    --freq "$PERF_FREQUENCY" \
    --call-graph "$PERF_CALL_GRAPH" \
    "${perf_delay_args[@]}" \
    -- "${application_command[@]}"
readonly APPLICATION_STATUS=$?
set -e

if [[ -s "$PERF_DATA" ]]; then
    printf 'Generating text report...\n'
    if ! perf report --stdio --input "$PERF_DATA" >"$PERF_REPORT"; then
        printf 'Warning: perf could not generate %s; raw data was preserved.\n' "$PERF_REPORT" >&2
    fi
else
    printf 'Error: perf did not produce profiling data. Check perf_event_paranoid and perf permissions.\n' >&2
fi

printf 'Profiling results: %s\n' "$RUN_DIR"
printf 'Interactive analysis: perf report --input %q\n' "$PERF_DATA"
if [[ -s "$RESOURCE_USAGE" ]]; then
    printf 'Resource usage: %s\n' "$RESOURCE_USAGE"
fi

exit "$APPLICATION_STATUS"
