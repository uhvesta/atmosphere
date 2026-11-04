"""Normalize Bazel's host OS and CPU names for local tool downloads."""

def host_binary_key(os):
    """Return the host key used by prebuilt tool release assets.

    Args:
        os: The Bazel module context's host operating system.

    Returns:
        A key such as ``darwin_arm64``.
    """
    name = os.name.lower()
    arch = os.arch.lower()

    if "mac" in name:
        system = "darwin"
    elif "linux" in name:
        system = "linux"
    else:
        fail("Unsupported operating system: {}".format(os.name))

    if arch in ["aarch64", "arm64"]:
        cpu = "arm64"
    elif arch in ["x86_64", "amd64"]:
        cpu = "amd64"
    else:
        fail("Unsupported CPU architecture: {}".format(os.arch))

    return "{}_{}".format(system, cpu)
