# Local OpenSSH test server

From the repository root on an Apple Silicon Mac:

```sh
./terminal-ssh-demo.sh start
./terminal-ssh-demo.sh provision
./terminal-ssh-demo.sh status
```

The Bash helper delegates all builds and automation to Bazel. `start`
creates the dedicated `h2-terminal-c1` Debian Lima VM if missing, starts it
if stopped, and checks its real OpenSSH service. A running VM is reused.
The fixture has one CPU, 0.75 GiB RAM, a 4 GiB disk, no host mounts, no
`containerd` and no SSH agent forwarding. Install Lima and Python 3 first.
The helper rejects an existing VM with a different fixture configuration.
It never stops, restarts, deletes or remounts a VM.

`provision` first builds the holder through Bazel's `linux_arm64`
configuration, then ensures the VM is running and atomically installs that
matching aarch64 Linux executable at `/tmp/h2-terminal-holder`. It validates
the ELF architecture before copying. Existing holder processes and their
shells continue running. Repeating provision replaces only the executable
used by future sessions; it does not upgrade an already running holder.
`/tmp` is suitable for this experiment, not machine-reboot persistence.

The server binds to loopback through Lima's generated SSH configuration.
The helper leaves host-key checks and authentication enabled; it does not
copy personal configuration files, history, SSH keys or credentials into the guest.
Read the configuration path without starting or modifying the VM:

```sh
./terminal-ssh-demo.sh ssh-config
ssh -F "$HOME/.lima/h2-terminal-c1/ssh.config" lima-h2-terminal-c1
```

Use the same configuration in the terminal lab:

```sh
bazel run //apps/humata2/terminal/lab:terminal_lab -- \
  --data /tmp/my-ssh-terminal-lab \
  --ssh-config "$HOME/.lima/h2-terminal-c1/ssh.config" \
  --ssh-host lima-h2-terminal-c1 \
  --remote-holder /tmp/h2-terminal-holder \
  --remote-root /var/tmp/my-ssh-terminal-lab
```

Choose **+ SSH**. For the persistent host, configure the same endpoint in
its private profiles file as described in [the host README](../host/README.md).
Reusing its private workspace reconnects existing holders. Closing the
browser or stopping the local app does not end the remote shell; stopping
the VM does. Do not restart this shared fixture while SSH acceptance or
live demo sessions are running.

A read-only real-SSH status check is available through Bazel:

```sh
bazel test //apps/humata2/terminal/platform_test:ssh_demo_status_test \
  --test_output=all
```

It reports a missing/stopped/unreachable fixture instead of creating one.
Browser preparation and the Linux Chromium proof are separate, explicit
targets documented in [README.md](README.md).
