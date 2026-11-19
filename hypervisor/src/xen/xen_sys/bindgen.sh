#!/usr/bin/env bash
# Copyright 2026 EPAM Systems
# Use of this source code is governed by a BSD-style license that can be
# found in the LICENSE file.

# Regenerate xen_sys bindgen bindings.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../../../.."

source tools/impl/bindgen-common.sh

bindgen_generate \
    "hypervisor/src/xen/xen_sys/xc_wrapper.h" \
    -- \
    -Ixen/dist/usr/local/include/xen \
    > hypervisor/src/xen/xen_sys/bindings.rs