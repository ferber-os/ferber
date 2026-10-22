#!/usr/bin/env bash
# Compatibility entry for the finite ELF regression fixture.
set -euo pipefail
exec "$(dirname "${BASH_SOURCE[0]}")/build-user.sh" elf-probe "$@"
