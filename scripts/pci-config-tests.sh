#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
test_dir=$(mktemp -d /tmp/ferber-pci-config-tests.XXXXXX)
trap 'rm -rf -- "$test_dir"' EXIT
rustc --edition=2021 -D warnings --test "$repo_root/scripts/pci-config-tests.rs" -o "$test_dir/tests"
"$test_dir/tests"
for level in 0 3; do
    rustc --edition=2021 -D warnings --crate-type lib --emit asm -C "opt-level=$level" \
        "$repo_root/scripts/pci-config-tests.rs" -o "$test_dir/code.s"
    python3 - "$test_dir/code.s" <<'PY'
import pathlib, re, sys
text = pathlib.Path(sys.argv[1]).read_text()
# Inspect the actual inline-assembly blocks, independent of allocator/register choices.
blocks = re.findall(r'#APP\n(.*?)#NO_APP', text, re.S)
for pattern in [r'movl\s+\(%\w+\), %eax', r'movl\s+%eax, \(%\w+\)', r'movw\s+%ax, \(%\w+\)']:
    assert any(re.search(pattern, b) for b in blocks), (pattern, blocks)
PY
done
echo 'PCI config accumulator code generation passed (debug and optimized)'
