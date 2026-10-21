#!/usr/bin/env bash
# Checks `orag-vulkan gpu-probe` against the Vulkan drivers the loader is
# allowed to see (v0.6 plan, Task 2; Linux CI with mesa-vulkan-drivers):
# - no driver: no devices, nothing chosen, exit 0;
# - lavapipe only (a software rasteriser, admitted with
#   GGML_VK_VISIBLE_DEVICES=0 because ggml skips CPU-type devices): one
#   device named llvmpipe, which is not chosen unless the test-only
#   ORAG_VULKAN_ALLOW_SOFTWARE=1 is set.
# Usage: scripts/check-gpu-probe.sh <orag-vulkan> [<lavapipe ICD json>]
set -euo pipefail
bin="${1:?usage: $0 <orag-vulkan> [<lavapipe ICD json>]}"
icd="${2:-/usr/share/vulkan/icd.d/lvp_icd.x86_64.json}"
fail() { echo "FAIL: $*" >&2; exit 1; }
[ -f "$icd" ] || fail "no lavapipe ICD at $icd (apt-get install mesa-vulkan-drivers)"

# VK_ICD_FILENAMES for older loaders (Ubuntu 22.04's 1.3.204), VK_DRIVER_FILES for newer.
probe() { VK_ICD_FILENAMES="$1" VK_DRIVER_FILES="$1" "${@:2}" "$bin" gpu-probe; }

none=$(probe /nonexistent/icd.json env)
[ "$(jq -c '[.backend, .devices, .chosen]' <<<"$none")" = '["vulkan",[],null]' ] \
  || fail "with no driver: $none"

soft=$(probe "$icd" env GGML_VK_VISIBLE_DEVICES=0)
[ "$(jq '.devices | length' <<<"$soft")" = 1 ] || fail "with lavapipe: $soft"
jq -e '.devices[0].name | test("llvmpipe")' <<<"$soft" >/dev/null || fail "not llvmpipe: $soft"
[ "$(jq '.chosen' <<<"$soft")" = null ] || fail "a software rasteriser was chosen: $soft"

allowed=$(probe "$icd" env GGML_VK_VISIBLE_DEVICES=0 ORAG_VULKAN_ALLOW_SOFTWARE=1)
[ "$(jq '.chosen == .devices[0].index' <<<"$allowed")" = true ] \
  || fail "lavapipe not chosen when allowed: $allowed"
echo "gpu-probe ok: $(jq -c '.devices[0] | {name, kind}' <<<"$soft")"
