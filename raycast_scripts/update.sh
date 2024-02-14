#!/usr/bin/env bash
set -euo pipefail
shopt -s inherit_errexit
DIR="$( cd "$( dirname "$(readlink "${BASH_SOURCE[0]}")" )" >/dev/null 2>&1 && pwd )"
cd "$DIR/.."

# @raycast.schemaVersion 1
# @raycast.title RTB Update
# @raycast.author William Goodall
# @raycast.authorURL https://williamgoodall.com
# @raycast.description Update Roam Third Brain from a JSON Zip export on the desktop
# @raycast.mode fullOutput


# Make a temp dir for scratch space, named 'rtb.XXXXX'
tmp_dir="$(mktemp -d -t rtb.XXXXX)"
echo "$0: using tmp_dir: $tmp_dir"

# Get the first filename matching `~/Desktop/Roam-Export*`, erroring if many are found.
export_files=("$HOME/Desktop/Roam-Export"*.zip)
if [[ "${#export_files[@]}" -gt 1 ]]; then
  echo "Found more than one Roam-Export*.zip file on the desktop. Please remove all but the latest."
  exit 1
fi

# Extract the 'Roam-Export*' zip file on the desktop to the temp dir
unzip -q -d "$tmp_dir" "${export_files[0]}"

# Source the env
source .env

# Get the bin
rtb="$DIR/../target/release/rtb"

# Import the latest backup
$rtb import "$tmp_dir"/*.json

# Update embeddings
$rtb update-embeddings

# Remove the temp
rm -rf "$tmp_dir"
