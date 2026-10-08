#!/bin/sh
# kipi:post-checkout (installed by `kipi hook install`; `kipi hook uninstall` removes it)
# Gives a new working tree (`git worktree add`) kipi's files with `kipi sync`. It never
# fails a checkout: kipi's errors go to stderr and its exit status is ignored. A hook
# that was here before kipi's is kept beside it as post-checkout.pre-kipi and run first.
status=0
chained="$(dirname "$0")/post-checkout.pre-kipi"
if [ -x "$chained" ]; then
  "$chained" "$@" || status=$?
fi
# Only a checkout from no commit at all (the all-zero id) makes a new working tree.
case "$1" in
  *[!0]* | "") ;;
  *)
    if command -v kipi >/dev/null 2>&1; then
      (unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE; kipi sync --hook --quiet >/dev/null) || true
    fi
    ;;
esac
exit $status
