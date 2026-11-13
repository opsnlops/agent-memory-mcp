#!/usr/bin/env bash
# Fail unless the generated manual page is complete.
#
#   scripts/check-manpage.sh <binary> <manpage>
#
# The fleet packaging policy requires every package to ship a man page that
# covers all command-line options and how to configure the program on the
# filesystem. help2man fails quietly (a renamed heading lands the options under
# DESCRIPTION, a lost include drops FILES), so debian/rules runs this right
# after generating the page.

set -euo pipefail

bin=$1
page=$2
status=0

for section in OPTIONS CONFIGURATION ENVIRONMENT FILES EXAMPLES; do
  if ! grep -q "^\.SH \"\?$section\"\?\$" "$page"; then
    echo "check-manpage: $page has no .SH $section section" >&2
    status=1
  fi
done

# Every long option in --help must appear in the page. groff escapes hyphens,
# so compare against the page with "\-" turned back into "-".
unescaped=$(sed 's/\\-/-/g' "$page")
options=$("$bin" --help | grep -o -- '--[a-z][a-z0-9-]*' | sort -u)
for opt in $options; do
  if ! grep -q -- "$opt" <<<"$unescaped"; then
    echo "check-manpage: $page does not mention $opt" >&2
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "check-manpage: $page ok ($(wc -w <<<"$options" | tr -d ' ') options)"
fi
exit "$status"
