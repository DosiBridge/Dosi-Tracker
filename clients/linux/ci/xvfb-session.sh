#!/usr/bin/env bash
#
# Runs a command inside a throwaway graphical session owned by this script:
# a private XDG_RUNTIME_DIR, an Xvfb display with the extensions the agent
# uses, an openbox window manager, a private session bus and a second private
# bus that stands in for the system bus. Nothing touches the caller's display,
# buses or keyring.
#
#   bash ci/xvfb-session.sh cargo test --locked --features test-support -- --ignored
#
# Tests find the harness through DOSI_TEST_HARNESS=1, DOSI_TEST_DISPLAY,
# DOSI_TEST_SESSION_BUS and DOSI_TEST_SYSTEM_BUS (see tests/support/xvfb.rs).
# The command's exit code is propagated; everything started here is torn down.

set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: bash ci/xvfb-session.sh <command> [args...]" >&2
  exit 2
fi

DISPLAY_NUM="${DOSI_XVFB_DISPLAY_NUM:-99}"
HARNESS_DISPLAY=":${DISPLAY_NUM}"

for tool in Xvfb xdpyinfo openbox dbus-daemon; do
  command -v "${tool}" >/dev/null 2>&1 || {
    echo "ci/xvfb-session.sh: '${tool}' is required but not installed" >&2
    exit 1
  }
done

RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/dosi-harness-XXXXXX")"
chmod 0700 "${RUNTIME_DIR}"

PIDS=()

cleanup() {
  local pid
  for ((i = ${#PIDS[@]} - 1; i >= 0; i--)); do
    pid="${PIDS[i]}"
    kill "${pid}" 2>/dev/null || true
  done
  for pid in "${PIDS[@]}"; do
    wait "${pid}" 2>/dev/null || true
  done
  rm -rf "${RUNTIME_DIR}"
  rm -f "/tmp/.X${DISPLAY_NUM}-lock" "/tmp/.X11-unix/X${DISPLAY_NUM}" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

# Starts a private bus of the given type and stores its address in the
# variable named by the second argument (no subshell, so the daemon's pid is
# tracked for teardown).
start_bus() {
  local kind="$1" out_var="$2"
  local dir="${RUNTIME_DIR}/${kind}-bus"
  mkdir -m 0700 "${dir}"
  cat >"${dir}/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>${kind}</type>
  <listen>unix:dir=${dir}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
  dbus-daemon --config-file="${dir}/bus.conf" --nofork --print-address=3 \
    3>"${dir}/address" 2>"${dir}/log" &
  PIDS+=("$!")
  local tries=0
  while [ ! -s "${dir}/address" ]; do
    tries=$((tries + 1))
    if [ "${tries}" -gt 100 ]; then
      echo "ci/xvfb-session.sh: private ${kind} bus did not start" >&2
      cat "${dir}/log" >&2 || true
      exit 1
    fi
    sleep 0.05
  done
  local address
  address="$(head -n 1 "${dir}/address")"
  printf -v "${out_var}" '%s' "${address}"
}

export XDG_RUNTIME_DIR="${RUNTIME_DIR}"
unset WAYLAND_DISPLAY DBUS_SESSION_BUS_ADDRESS DBUS_SYSTEM_BUS_ADDRESS XAUTHORITY

Xvfb "${HARNESS_DISPLAY}" -screen 0 1920x1080x24 -nolisten tcp -noreset \
  +extension XInputExtension +extension XTEST +extension MIT-SCREEN-SAVER +extension RANDR \
  >"${RUNTIME_DIR}/xvfb.log" 2>&1 &
PIDS+=("$!")

tries=0
until xdpyinfo -display "${HARNESS_DISPLAY}" >/dev/null 2>&1; do
  tries=$((tries + 1))
  if [ "${tries}" -gt 100 ]; then
    echo "ci/xvfb-session.sh: Xvfb did not come up on ${HARNESS_DISPLAY}" >&2
    cat "${RUNTIME_DIR}/xvfb.log" >&2 || true
    exit 1
  fi
  sleep 0.1
done
export DISPLAY="${HARNESS_DISPLAY}"

SESSION_BUS=""
SYSTEM_BUS=""
start_bus session SESSION_BUS
start_bus system SYSTEM_BUS
export DBUS_SESSION_BUS_ADDRESS="${SESSION_BUS}"
export DBUS_SYSTEM_BUS_ADDRESS="${SYSTEM_BUS}"

openbox >"${RUNTIME_DIR}/openbox.log" 2>&1 &
PIDS+=("$!")

export DOSI_TEST_HARNESS=1
export DOSI_TEST_DISPLAY="${HARNESS_DISPLAY}"
export DOSI_TEST_SESSION_BUS="${SESSION_BUS}"
export DOSI_TEST_SYSTEM_BUS="${SYSTEM_BUS}"

status=0
"$@" || status=$?
exit "${status}"
