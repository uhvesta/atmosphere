#!/bin/sh
# The fake `gh` of pr_demo, a copy of FAKE_GH in
# apps/humata2/daemon/humatad/tests/common/pr_github.rs: keep the two alike.
# It answers from files beside it (env, visible/, prs/, graphql.json) and
# never reaches GitHub or the real `gh` login.
here=$(dirname "$0")
. "$here/env"
printf '%s\n' "$*" >> "$here/gh.log"
if [ -n "${GH_TOKEN}${GITHUB_TOKEN}${GH_ENTERPRISE_TOKEN}" ]; then
  echo "a token in the environment" >> "$here/tokens.log"
fi
for arg in "$@"; do
  if [ "$arg" = "--with-token" ]; then echo "--with-token" >> "$here/tokens.log"; fi
done

unexpected() { echo "fake gh: unexpected $*" >&2; exit 1; }

fail_with() {
  case "$1" in
    signed-out)
      echo "To get started with GitHub CLI, please run:  gh auth login" >&2; exit 4 ;;
    rate-limited)
      echo "GraphQL: API rate limit exceeded for user ID 1. (HTTP 403)" >&2; exit 1 ;;
    bad-line)
      echo '{"message":"Unprocessable Entity","errors":["Pull request review thread line must be part of the diff"],"status":"422"}'
      echo "gh: Unprocessable Entity (HTTP 422)" >&2; exit 1 ;;
  esac
  echo "fake gh: unknown failure $1" >&2; exit 1
}

list() {
  printf '['
  sep=
  for f in "$1"/*.json; do
    [ -e "$f" ] || continue
    printf '%s' "$sep"; cat "$f"; sep=,
  done
  printf ']\n'
}

pr_view() {
  target=$1; shift
  repo=
  while [ $# -gt 0 ]; do
    case "$1" in
      --repo|-R) repo=$2; shift ;;
      --repo=*) repo=${1#--repo=} ;;
    esac
    shift
  done
  case "$target" in
    https://github.com/*)
      rest=${target#https://github.com/}
      repo=${rest%%/pull/*}
      target=${rest#*/pull/}
      target=${target%%[/?#]*} ;;
  esac
  owner=${repo%%/*}; name=${repo#*/}
  if [ ! -e "$here/visible/$owner-$name" ]; then
    echo "GraphQL: Could not resolve to a Repository with the name '$repo'. (repository)" >&2; exit 1
  fi
  if [ ! -e "$here/prs/$owner-$name-$target.json" ]; then
    echo "GraphQL: Could not resolve to a PullRequest with the number of $target. (repository.pullRequest)" >&2; exit 1
  fi
  cat "$here/prs/$owner-$name-$target.json"
}

api_call() {
  path=; post=; input=; jq=; graphql=; user=; prev=
  for arg in "$@"; do
    case "$prev" in
      --input) input=$arg ;;
      -X|--method) [ "$arg" = POST ] && post=1 ;;
    esac
    case "$arg" in
      --method=POST|-XPOST|--input) post=1 ;;
      --jq|-q) jq=1 ;;
      graphql) graphql=1 ;;
      user) user=1 ;;
      repos/*) path=$arg ;;
      /repos/*) path=${arg#/} ;;
    esac
    prev=$arg
  done
  if [ -n "$graphql" ]; then
    # While the hold is in place the answer waits, as a slow GitHub would
    # (bounded, so a test that forgets to lift it still ends).
    waited=0
    while [ -e "$here/graphql-hold" ] && [ "$waited" -lt 1200 ]; do
      sleep 0.05; waited=$((waited + 1))
    done
    if [ -e "$here/graphql.json" ]; then cat "$here/graphql.json"; else echo '{"data":{}}'; fi
    exit 0
  fi
  if [ -n "$user" ] && [ -z "$path" ]; then
    if [ -n "$jq" ]; then echo "$FAKE_LOGIN"; else echo "{\"login\":\"$FAKE_LOGIN\"}"; fi
    exit 0
  fi
  [ -n "$path" ] || unexpected "api $*"
  path=${path%%\?*}
  IFS=/ read -r _repos owner name _pulls number tail <<EOF
$path
EOF
  key="$owner-$name-$number"
  case "$tail" in
    reviews)
      if [ -n "$post" ]; then
        if [ -s "$here/fail-post-$key" ]; then fail_with "$(cat "$here/fail-post-$key")"; fi
        seq=$(ls "$here/posted" | wc -l | tr -d ' ')
        id=$((1001 + seq))
        if [ -z "$input" ] || [ "$input" = "-" ]; then
          cat > "$here/posted/$key.$id.json"
        else
          cat "$input" > "$here/posted/$key.$id.json"
        fi
        echo "{\"id\":$id,\"html_url\":\"https://github.com/$owner/$name/pull/$number#pullrequestreview-$id\",\"state\":\"COMMENTED\",\"user\":{\"login\":\"$FAKE_LOGIN\"}}"
      else
        list "$here/listed/$key"
      fi ;;
    reviews/*/comments)
      id=${tail#reviews/}; id=${id%/comments}
      if [ -e "$here/review-comments/$id.json" ]; then cat "$here/review-comments/$id.json"; else echo '[]'; fi ;;
    *) unexpected "api $*" ;;
  esac
}

case "$1 $2" in
  "auth token") echo "$FAKE_TOKEN"; exit 0 ;;
  "auth status")
    echo "github.com"
    echo "  Logged in to github.com account $FAKE_LOGIN (keyring)"
    echo "  Token: $FAKE_TOKEN"
    exit 0 ;;
  "api rate_limit")
    echo '{"resources":{"core":{"limit":5000,"remaining":0,"reset":4102444800},"graphql":{"limit":5000,"remaining":0,"reset":4102444800}},"rate":{"limit":5000,"remaining":0,"reset":4102444800}}'
    exit 0 ;;
esac
if [ -s "$here/fail" ]; then fail_with "$(cat "$here/fail")"; fi
case "$1" in
  pr)
    case "$2" in
      view) shift 2; pr_view "$@" ;;
      list) echo '[]' ;;
      *) unexpected "$@" ;;
    esac ;;
  api) shift; api_call "$@" ;;
  *) unexpected "$@" ;;
esac
