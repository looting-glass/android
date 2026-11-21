#!/bin/sh
# Score the parser on the saved ML Kit OCR of every test set and fail if a set scores below
# testdata/baseline.txt. No phone or OCR needed: the OCR output is in testdata/mlkit/.
set -e
cd "$(dirname "$0")/.."
fail=0
while read -r set min; do
    case "$set" in ''|\#*) continue ;; esac
    out=$(cargo run -q --release --locked -p lootlens-eval -- testdata/items "testdata/mlkit/$set")
    total=$(printf '%s\n' "$out" | grep '^TOTAL')
    got=$(printf '%s\n' "$total" | sed -n 's/.*mods correct \([0-9]*\)\/.*/\1/p')
    echo "$set: $total"
    if [ -z "$got" ] || [ "$got" -lt "$min" ]; then
        echo "::error::$set: ${got:-?} mods correct, baseline is $min"
        printf '%s\n' "$out"
        fail=1
    fi
done < testdata/baseline.txt
exit $fail
