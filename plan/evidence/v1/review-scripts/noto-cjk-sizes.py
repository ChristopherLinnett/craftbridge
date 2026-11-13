"""Re-fetches the sizes, licence and naming evidence behind RESULT §4's Chinese font proposal,
without downloading any font: eight GitHub contents-API listings ("<size in bytes> <type> <name>"
per entry) and excerpts of four text files: the licence, the two METADATA.pb files and the
upstream note. Writes <output> only after every fetch succeeded, so
a refused request (the GitHub API allows 60 unauthenticated requests an hour per address) leaves
the previous capture as it was.
Usage: python3 -I review-scripts/noto-cjk-sizes.py capture/review-noto-cjk-sizes.txt"""
import json
import os
import sys
import time
import urllib.error
import urllib.request

API = "https://api.github.com/repos/"
RAW = "https://raw.githubusercontent.com/"
LISTINGS = [
    "notofonts/noto-cjk/contents/Sans/SubsetOTF/SC",
    "notofonts/noto-cjk/contents/Sans/SubsetOTF/TC",
    "notofonts/noto-cjk/contents/Sans/Variable/OTF/Subset",
    "notofonts/noto-cjk/contents/Sans/Variable/TTF/Subset",
    "notofonts/noto-cjk/contents/Sans/OTF/SimplifiedChinese",
    "notofonts/noto-cjk/contents/Sans/OTF/TraditionalChinese",
    "google/fonts/contents/ofl/notosanssc",
    "google/fonts/contents/ofl/notosanstc",
]


def fetch(url):
    try:
        with urllib.request.urlopen(url, timeout=30) as r:
            return r.read().decode()
    except urllib.error.HTTPError as e:
        left = e.headers.get("X-RateLimit-Remaining")
        hint = " (GitHub's unauthenticated rate limit is used up; try again after it resets)" if left == "0" else ""
        sys.exit("%s: HTTP %d %s%s; output not written" % (url, e.code, e.reason, hint))
    except urllib.error.URLError as e:
        sys.exit("%s: %s; output not written" % (url, e.reason))


def listing(path):
    return "".join("%10d %s %s\n" % (e["size"], e["type"], e["name"]) for e in json.loads(fetch(API + path)))


def metadata_excerpt(text):
    """The family name, the licence, and the first font entry's name, weight, file and PostScript name."""
    out, in_font = [], False
    for line in text.splitlines(keepends=True):
        if line.startswith(("name: ", "license: ")):
            out.append(line)
        elif line.startswith("fonts {"):
            in_font = True
        elif in_font and line.startswith("}"):
            in_font = False
        elif in_font and line.startswith(("  name: ", "  weight: ", "  filename: ", "  post_script_name: ")):
            out.append(line)
    return "".join(out)


def report():
    parts = ["# GitHub contents API, fetched %s; sizes in bytes\n" % time.strftime("%Y-%m-%dT%H:%MZ", time.gmtime())]
    for path in LISTINGS:
        parts += ["## %s%s\n" % (API, path), listing(path)]
    licence = RAW + "notofonts/noto-cjk/main/Sans/LICENSE"
    parts += ["## %s (first 2 lines)\n" % licence, "".join(fetch(licence).splitlines(keepends=True)[:2])]
    for family in ("notosanssc", "notosanstc"):
        url = RAW + "google/fonts/main/ofl/%s/METADATA.pb" % family
        parts += ["## %s (name, license, font file and default instance)\n" % url, metadata_excerpt(fetch(url))]
    upstream = RAW + "google/fonts/main/ofl/notosanssc/upstream_info.md"
    parts.append("## %s (source of the Google Fonts binary)\n" % upstream)
    parts += [line + "\n" for line in fetch(upstream).splitlines() if "regional subset" in line][:1]
    return "".join(parts)


if len(sys.argv) != 2:
    sys.exit(__doc__)
text = report()
tmp = sys.argv[1] + ".tmp"
with open(tmp, "w") as f:
    f.write(text)
os.replace(tmp, sys.argv[1])
