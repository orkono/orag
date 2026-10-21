#!/usr/bin/env bash
# Checks `orag-vulkan gpu-probe --load` on Mesa's lavapipe, the only Vulkan
# device CI has (v0.6 plan, Task 3): every branch of the placed load.
# - fitted: layers on the Vulkan device, Vulkan buffers in llama.cpp's log;
# - ORAG_VULKAN_TEST_FIT=failure | zero_layers | alloc_failure: a CPU-only
#   load with that reason, and no Vulkan buffer after the CPU-only load
#   begins (an alloc_failure first loads on the GPU, then retries);
# - software device not allowed: a CPU-only load, no Vulkan buffer at all;
# - an ORAG_GPU_DEVICE that is not listed: a startup error naming it.
# Usage: scripts/check-gpu-offload.sh <orag-vulkan> <gguf> [<lavapipe ICD json>]
set -euo pipefail
bin="${1:?usage: $0 <orag-vulkan> <gguf> [<icd>]}"
gguf="${2:?usage: $0 <orag-vulkan> <gguf> [<icd>]}"
icd="${3:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}"
fail() { echo "FAIL: $*" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
vulkan_buffers='Vulkan[0-9_A-Za-z]* (model|compute|KV|output)'

# load NAME [VAR=VALUE...]: runs the load with lavapipe as the only device;
# stdout in $tmp/NAME.json, stderr in $tmp/NAME.log.
load() {
  local name="$1"; shift
  env VK_ICD_FILENAMES="$icd" VK_DRIVER_FILES="$icd" GGML_VK_VISIBLE_DEVICES=0 "$@" \
    "$bin" gpu-probe --load "$gguf" >"$tmp/$name.json" 2>"$tmp/$name.log"
}
field() { jq -r ".offload.$2" "$tmp/$1.json"; }
# Lines of the log after the CPU-only load began.
after_cpu_load() { sed -n '/gpu_offload=none: cpu-only load/,$p' "$tmp/$1.log"; }

load fitted ORAG_VULKAN_ALLOW_SOFTWARE=1 || fail "fitted load failed: $(tail -5 "$tmp/fitted.log")"
[ "$(field fitted layers_on_gpu)" -gt 0 ] || fail "nothing on the GPU: $(cat "$tmp/fitted.json")"
[ "$(field fitted reason)" = null ] || fail "fitted load has a reason: $(cat "$tmp/fitted.json")"
field fitted device | grep -q llvmpipe || fail "not on lavapipe: $(cat "$tmp/fitted.json")"
grep -qE "$vulkan_buffers" "$tmp/fitted.log" || fail "no Vulkan buffer in the fitted load's log"

for hook in failure:fit_failed zero_layers:fit_zero_layers alloc_failure:alloc_failed; do
  name="${hook%%:*}"; reason="${hook##*:}"
  load "$name" ORAG_VULKAN_ALLOW_SOFTWARE=1 ORAG_VULKAN_TEST_FIT="$name" \
    || fail "$name load failed: $(tail -5 "$tmp/$name.log")"
  [ "$(field "$name" layers_on_gpu)" = 0 ] || fail "$name: layers on the GPU: $(cat "$tmp/$name.json")"
  [ "$(field "$name" reason)" = "$reason" ] || fail "$name: reason: $(cat "$tmp/$name.json")"
  [ -n "$(after_cpu_load "$name")" ] || fail "$name: no cpu-only load in the log"
  if after_cpu_load "$name" | grep -E "$vulkan_buffers"; then
    fail "$name: Vulkan buffers after the CPU-only load began"
  fi
done

load software_refused
[ "$(field software_refused reason)" = no_eligible_device ] \
  || fail "lavapipe was used without the test switch: $(cat "$tmp/software_refused.json")"
if grep -E "$vulkan_buffers" "$tmp/software_refused.log"; then
  fail "Vulkan buffers in a CPU-only load"
fi

if load bad_device ORAG_GPU_DEVICE=9; then
  fail "ORAG_GPU_DEVICE=9 was accepted"
fi
grep -q "ORAG_GPU_DEVICE" "$tmp/bad_device.log" || fail "the error does not name ORAG_GPU_DEVICE: $(tail -3 "$tmp/bad_device.log")"
echo "gpu offload ok: fitted $(field fitted layers_on_gpu)/$(field fitted layers_total) layers on $(field fitted device)"
