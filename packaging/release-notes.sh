#!/usr/bin/env bash
# Prints the release description of <version> in English from CHANGELOG.md.
#
#   packaging/release-notes.sh 0.1.0-beta > notes.md
set -euo pipefail

version="${1:?usage: packaging/release-notes.sh <version>}"
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

changes="$(awk -v heading="## $version" '
    /^## / {
        if (found) exit
        found = ($0 == heading || index($0, heading " ") == 1)
        next
    }
    found
' "$root/CHANGELOG.md" | sed '/./,$!d' | awk '
    function flush() { if (have) print line; have = 0 }
    /^$/ { flush(); print ""; next }
    /^(#|- |[0-9]+\. |\||>)/ { flush(); line = $0; have = 1; next }
    { sub(/^ +/, ""); if (have) line = line " " $0; else { line = $0; have = 1 } }
    END { flush() }
')"
if [[ -z "${changes//[[:space:]]/}" ]]; then
    echo "CHANGELOG.md has no section for $version" >&2
    exit 1
fi

if [[ "$version" == *-* ]]; then
    kind='Prerelease'
else
    kind='Release'
fi

deb_v="${version//-/.}"

cat <<EOF
$kind for Linux (Ubuntu 24.04+, Debian 13+, amd64) and Windows (10/11, x64).

- **pwtool_${deb_v}_amd64.deb** — Debian/Ubuntu package: \`sudo apt install ./pwtool_${deb_v}_amd64.deb\` installs **Password Tool**, its desktop entry, icons, translations, script examples, and user manuals (\`manual.pdf\`, \`scripting.pdf\`).
- **pwtool_${deb_v}_windows_x64.zip** — portable Windows build with GTK 4: unpack anywhere and run \`PwTool\bin\pwtool.exe\`.

$changes

Settings are stored in \`~/.config/pwtool/PwTool.ini\` on Linux and \`%APPDATA%\PwTool\PwTool.ini\` on Windows (compatible with Password Tech's \`PwTech.ini\`). Place \`PwTool.ini\` next to the executable for portable mode.

License: GNU GPL v2 or later (Password Tech © 2002–2026 Christian Thöing). File checksums are in \`SHA256SUMS.txt\`.
EOF
