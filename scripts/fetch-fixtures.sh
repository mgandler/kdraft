#!/usr/bin/env bash
# Fetch KiCad demo boards from the KiCad source mirror into fixtures-ext/.
# Uses a shallow, blobless, sparse clone per tag so only demos/ is downloaded.
# fixtures-ext/ is git-ignored and must never be committed.
#
# Usage: scripts/fetch-fixtures.sh [TAG9] [TAG10]
set -euo pipefail

REPO_URL="${KDRAFT_KICAD_MIRROR:-https://github.com/KiCad/kicad-source-mirror.git}"
TAG9="${1:-9.0.9}"
TAG10="${2:-10.0.8}"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/fixtures-ext"
work="$dest/.src"
mkdir -p "$work"

fetch() {
    local tag="$1" major="$2"
    local clone="$work/$tag"
    if [ ! -d "$clone/.git" ]; then
        rm -rf "$clone"
        git clone --quiet --depth 1 --filter=blob:none --sparse --no-checkout \
            --branch "$tag" "$REPO_URL" "$clone"
        git -C "$clone" checkout --quiet "$tag"
    fi
    git -C "$clone" sparse-checkout set --no-cone \
        '/demos/**/*.kicad_pcb' '/demos/**/*.kicad_pro' '/demos/**/*.kicad_wks' '/template/**/*.kicad_wks'
    local out="$dest/$major"
    rm -rf "$out"
    mkdir -p "$out"
    # Flatten demos/<a>/<b>/name.ext into <a>__<b>__name.ext.
    (cd "$clone/demos" && find . \( -name '*.kicad_pcb' -o -name '*.kicad_pro' -o -name '*.kicad_wks' \) -print0) |
        while IFS= read -r -d '' f; do
            rel="${f#./}"
            cp "$clone/demos/$rel" "$out/${rel//\//__}"
        done
    if [ -d "$clone/template" ]; then
        (cd "$clone/template" && find . -name '*.kicad_wks' -print0) |
            while IFS= read -r -d '' f; do
                rel="${f#./}"
                cp "$clone/template/$rel" "$out/template__${rel//\//__}"
            done
    fi
    echo "$tag" > "$out/TAG"
    echo "fetched $(find "$out" -name '*.kicad_pcb' | wc -l) boards, $(find "$out" -name '*.kicad_pro' | wc -l) projects, $(find "$out" -name '*.kicad_wks' | wc -l) sheets for $tag into $out"
}

fetch "$TAG9" 9
fetch "$TAG10" 10
