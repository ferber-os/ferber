#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-iommu-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
rustc --edition=2021 -D warnings --test "$repo_root/scripts/iommu-tests.rs" -o "$test_dir/iommu-tests"
"$test_dir/iommu-tests"

# Exercise the production owner/drop path with fallible MMIO and allocation shims.
python3 - "$repo_root" "$test_dir" <<'PYTEST'
from pathlib import Path
import json
import sys
repo, output = map(Path, sys.argv[1:])
source = (repo / 'ferber/src/arch/x86_64/platform/iommu.rs').read_text()
marker = '/// Trusted control over the complete requester/alias assignment unit.'
assert source.count(marker) == 1
owner = source.split(marker)[0]
relative = '#[path = "../../../../../shared/amdvi.rs"]'
assert owner.count(relative) == 1
owner = owner.replace(relative, '#[path = ' + json.dumps(str(repo / 'shared/amdvi.rs')) + ']')
actual = output / 'coherent-production.rs'
actual.write_text(owner)
harness = (repo / 'scripts/iommu-coherent-tests.rs').read_text()
assert harness.count('"COHERENT_PRODUCTION_SOURCE"') == 1
harness = harness.replace('"COHERENT_PRODUCTION_SOURCE"', json.dumps(str(actual)))
(output / 'coherent-tests.rs').write_text(harness)
PYTEST
rustc --edition=2021 -D warnings --test "$test_dir/coherent-tests.rs" -o "$test_dir/coherent-tests"
"$test_dir/coherent-tests"
