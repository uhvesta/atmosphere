# Linux browser acceptance

Use [the root SSH helper](SSH_DEMO.md) to start or provision the dedicated
OpenSSH fixture. Its idempotent start and real SSH status checks passed
without restarting the existing VM.

This manual proof runs actual Chromium in the existing `h2-terminal-c1`
Debian Lima VM. The same Mac terminal host continues to own both its local
shell and its SSH shell. A temporary SSH reverse forward connects the VM's
loopback port 8741 to the Mac host's loopback port 8741. The VM receives only
the browser smoke script, private viewer URL and session expectations.
No VM is started, stopped, restarted or remounted.

Prepare the browser runtime once through Bazel:

```sh
bazel test //apps/humata2/terminal/platform_test:prepare_linux_browser_test \
  --strategy=TestRunner=local \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_output=all
```

The target installs Debian's Chromium, Node and software `Vulkan` packages
only when missing. It checks Node's built-in WebSocket first, then tries
Node 20's `--experimental-websocket`; no `npm` dependency or extra runtime
download is used. It leaves live SSH processes running.

Runtime preparation passed on Debian 13 arm64 with Chromium
156.0.8078.79 and Node 20.19.2. That Node version supplies the required
WebSocket API with `--experimental-websocket`.

Once the Mac gateway and browser bundle are running, use their private
`viewer.url` file and the actual stable session IDs:

```sh
bazel test //apps/humata2/terminal/platform_test:linux_browser_test \
  --strategy=TestRunner=local \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_env=TERMINAL_VIEWER_URL_FILE=/private/path/to/viewer.url \
  --test_env=TERMINAL_EXPECT_SESSION_OS='{"1":"Darwin","2":"Linux"}' \
  --test_output=all
```

The browser test uses a fresh temporary profile and the shared GPUI smoke
script. It sends real mouse/keyboard input to both panes, verifies distinct
command output followed by the expected `uname`, then reloads the same
live sessions. Chromium uses software `Vulkan` in the small VM; this proves
cross-OS behavior rather than hardware GPU performance. Mirror acceptance
is optional with `TERMINAL_BROWSER_MIRROR=1` and uses more memory.

Screenshots, browser state/version JSON and runtime diagnostics are copied
to Bazel's undeclared test outputs. URL/token values travel through standard input
and private files, and text diagnostics are scrubbed before collection.
Cleanup terminates only the owned browser process group and SSH reverse
forward, then removes the owned temporary directory. It never stops a
holder or shell process. The acceptance runner changes no swap or VM
configuration.

The read-only graphics inventory target helps select a software renderer:

```sh
bazel test //apps/humata2/terminal/platform_test:linux_graphics_probe_test \
  --test_env=TERMINAL_LAB_SSH_CONFIG="$HOME/.lima/h2-terminal-c1/ssh.config" \
  --test_output=all
```

The verified Debian Chromium package contains `libvk_swiftshader.so`, its
SwiftShader ICD, `libEGL.so` and `libGLESv2.so`. Mesa 25.0.7 also supplies
`swrast_dri.so`, `libvulkan_lvp.so` and the `Lavapipe` ICD. The probe changes
no packages, graphics configuration or running processes. Actual terminal
rendering is verified separately by the browser acceptance target.

The prepared Linux runner defaults to WebGL-only SwiftShader and supplies
`backend=webgl` when the viewer URL has no explicit backend. The optimized full fixture passes local/SSH
clipboard and reload on the unchanged721MiB/no-swap VM. Software Vulkan
previously painted black, and debug Wasm builds OOM-killed Chromium during
reload. Build milestone assets with `-c opt`; do not change VM resources or
extend timeouts to hide an acceptance failure. `TERMINAL_BROWSER_FLAGS` can
explicitly override the runner flags for graphics diagnostics.
