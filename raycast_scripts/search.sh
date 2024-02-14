#!/usr/bin/env bash
set -euo pipefail
shopt -s inherit_errexit
DIR="$( cd "$( dirname "$(readlink "${BASH_SOURCE[0]}")" )" >/dev/null 2>&1 && pwd )"
cd "$DIR/.."

# @raycast.schemaVersion 1
# @raycast.title RTB Search
# @raycast.author William Goodall
# @raycast.authorURL https://williamgoodall.com
# @raycast.description Search for results in Roam Third Brain.
# @raycast.mode fullOutput
# @raycast.argument1  {"type": "text", "placeholder": "Query"}

source ".env"

# Make a temp file for the answer
answer="$(mktemp -t rtb_answer_XXXXXX.md)"

./target/release/rtb search -o "$answer" -- "$*"

echo
echo

cat "$answer"
pbcopy <"$answer"
rm "$answer"
