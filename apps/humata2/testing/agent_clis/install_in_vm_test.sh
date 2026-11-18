#!/usr/bin/env bash
# shellcheck shell=bash
# install_in_vm.sh against a fake limactl: it copies only the two CLIs into
# ~/.humata2/test-bin, and refuses a bad VM name or arch before copying.
set -euo pipefail
script="$1"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
log="$work/calls"
: >"$log"
for f in c-arm c-x64 x-arm x-x64; do echo "$f" >"$work/$f"; done

cat >"$work/limactl" <<FAKE
#!/usr/bin/env bash
echo "\$*" >>"$log"
case "\$1" in
  list) echo Running ;;
  shell)
    shift 3
    if [[ "\$1" == uname ]]; then echo aarch64; fi
    if [[ "\$1" == sh ]]; then printf /home/me.linux; fi
    ;;
esac
FAKE
chmod +x "$work/limactl"
export H2_LIMACTL="$work/limactl"
args=("$work/c-arm" "$work/c-x64" "$work/x-arm" "$work/x-x64")

out="$(bash "$script" "${args[@]}" vm1)"
copies="$(grep '^copy ' "$log")"
[[ "$(wc -l <<<"$copies")" -eq 2 ]] || { echo "want two copies, got: $copies"; exit 1; }
grep -q "c-arm vm1:/home/me.linux/.humata2/test-bin/copilot$" <<<"$copies" || { echo "arch autodetect did not pick the arm64 copilot: $copies"; exit 1; }
grep -q "x-arm vm1:/home/me.linux/.humata2/test-bin/codex$" <<<"$copies" || { echo "arch autodetect did not pick the arm64 codex: $copies"; exit 1; }
[[ "$(grep -c "vm1:/home/me.linux/.humata2/test-bin/" <<<"$out")" -eq 2 ]] || { echo "want two installed paths printed: $out"; exit 1; }
if grep '^copy ' "$log" | grep -v '/.humata2/test-bin/\(copilot\|codex\)$' | grep -q .; then echo "copied outside test-bin"; exit 1; fi

: >"$log"
bash "$script" "${args[@]}" vm1 x64 >/dev/null
grep -q "c-x64 vm1:" "$log" || { echo "explicit x64 was not honoured"; exit 1; }

: >"$log"
for bad in 'vm;rm -rf' '../x' '-flag' 'a b'; do
  if bash "$script" "${args[@]}" "$bad" >/dev/null 2>&1; then echo "accepted VM name '$bad'"; exit 1; fi
done
if bash "$script" "${args[@]}" vm1 riscv >/dev/null 2>&1; then echo "accepted unknown arch"; exit 1; fi
if grep -q '^copy ' "$log"; then echo "copied despite bad input"; exit 1; fi
echo PASS
