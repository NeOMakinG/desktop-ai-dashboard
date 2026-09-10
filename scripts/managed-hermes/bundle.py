"""Build-time checks only. Never imports or executes Hermes or the controller."""
from __future__ import annotations

import hashlib
import importlib.metadata
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


def require(value, message):
    if not value:
        raise RuntimeError(message)


def inside(path, root):
    path = path.resolve()
    require(path.is_relative_to(root.resolve()), f"Path escaped bundle: {path}")
    return path


def inventory(root):
    result = []
    for path in sorted(root.rglob("*")):
        require(not path.is_symlink(), f"Unexpected symlink: {path}")
        if path.is_file():
            with path.open("rb") as stream:
                digest = hashlib.file_digest(stream, "sha256").hexdigest()
            result.append({"path": path.relative_to(root).as_posix(), "sha256": digest})
    return result


def materialize(root):
    # Tauri resource copying need not preserve symlinks. Eliminate that dependency
    # while refusing an archive link which could copy files from the build host.
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            target = inside(path, root)
            require(target.is_file(), f"Non-file archive symlink: {path}")
            data, mode = target.read_bytes(), target.stat().st_mode & 0o777
            path.unlink()
            path.write_bytes(data)
            path.chmod(mode)


def finalize(root):
    materialize(root)
    python = root / "python"
    source = root / "source"
    licenses = root / "licenses"
    licenses.mkdir(exist_ok=True)
    shutil.copy2(source / "LICENSE", licenses / "Hermes-MIT.txt")
    site = python / "lib/python3.12/site-packages"
    distributions = []
    for dist in importlib.metadata.distributions(path=[str(site)]):
        files = []
        for relative in dist.files or []:
            name = str(relative)
            # Preserve all wheel metadata and upstream license files in place;
            # also collect a browsable, separate third-party notices directory.
            if any(token in relative.name.lower() for token in ("license", "licence", "copying", "copyright", "notice", "authors")):
                original = inside(Path(dist.locate_file(relative)), root)
                if original.is_file():
                    destination = licenses / "wheels" / dist.metadata["Name"] / name.replace("../", "")
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(original, destination)
                    files.append(destination.relative_to(root).as_posix())
        require(files, f"Missing wheel license notices: {dist.metadata['Name']}")
        distributions.append({"name": dist.metadata["Name"], "version": dist.version,
                              "license": dist.metadata.get("License-Expression") or dist.metadata.get("License"),
                              "notices": files})
    (licenses / "wheels.json").write_text(json.dumps(distributions, indent=2) + "\n", encoding="utf-8")
    # Console-script shebangs contain build paths. Runtime invokes the pinned
    # interpreter explicitly; no pip/uv or ambient command dispatch is shipped.
    for item in (python / "bin").iterdir():
        if item.name != "python3.12":
            if item.is_dir():
                shutil.rmtree(item)
            else:
                item.unlink()
    for item in site.glob("pip*"):
        if item.is_dir():
            shutil.rmtree(item)
    for item in sorted(root.rglob("__pycache__"), reverse=True):
        shutil.rmtree(item)
    # Source is an unmodified public export, never a developer checkout/profile.
    # Environment templates are unnecessary in an app-owned, configless runtime.
    for item in source.rglob("*"):
        if item.is_file() and (item.name.startswith(".env") or item.suffix in (".pyc", ".pyo")):
            item.unlink()
    pins = json.loads((root / "build-pins.json").read_text(encoding="utf-8"))
    proof = {"revision": pins["hermes"]["commit"], "files": inventory(source)}
    (root / "source-proof.json").write_text(json.dumps(proof, indent=2) + "\n", encoding="utf-8")


def macho_load_commands(lines):
    rpaths, dependencies = [], []
    for i, line in enumerate(lines):
        command = line.strip()
        if command == "cmd LC_RPATH":
            rpaths.append(lines[i + 2].strip().removeprefix("path ").split(" (offset ")[0])
        elif command in {"cmd LC_LOAD_DYLIB", "cmd LC_LOAD_WEAK_DYLIB", "cmd LC_REEXPORT_DYLIB",
                         "cmd LC_LAZY_LOAD_DYLIB", "cmd LC_LOAD_UPWARD_DYLIB"}:
            dependencies.append(lines[i + 2].strip().removeprefix("name ").split(" (offset ")[0])
        # LC_ID_DYLIB is the image's identity, not a library loaded from PATH.
    return rpaths, dependencies


def macho_dependencies(root):
    allowed_os = ("/System/Library/", "/usr/lib/")
    executable = root / "python/bin"
    magics = {b"\xfe\xed\xfa\xce", b"\xce\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xcf\xfa\xed\xfe",
              b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca", b"\xca\xfe\xba\xbf", b"\xbf\xba\xfe\xca"}
    pinned = json.loads((root / "build-pins.json").read_text(encoding="utf-8"))
    inert_pins = pinned["platforms"]["darwin-arm64"].get("inertRpaths", [])
    ignored_rpaths = []
    checked = 0
    for binary in root.rglob("*"):
        if not binary.is_file():
            continue
        with binary.open("rb") as stream:
            if stream.read(4) not in magics:
                continue
        checked += 1
        def command(*args):
            return subprocess.check_output(["/usr/bin/otool", *args, str(binary)], text=True)
        loads = command("-l").splitlines()
        rpaths, dependencies = macho_load_commands(loads)
        def expand(value):
            value = value.replace("@loader_path", str(binary.parent)).replace("@executable_path", str(executable))
            return inside(Path(value), root)
        for value in rpaths:
            inert = next((entry for entry in inert_pins
                          if entry["file"] == binary.relative_to(root).as_posix() and entry["path"] == value), None)
            if inert:
                require(hashlib.sha256(binary.read_bytes()).hexdigest() == inert["sha256"], "Inert RPATH image changed")
                require(sorted(dependencies) == sorted(inert["loads"])
                        and all(item.startswith(allowed_os) for item in dependencies), "Inert RPATH became active")
                ignored_rpaths.append(inert)
                continue
            require(not value.startswith(allowed_os), f"External rpath: {binary}: {value}")
            expand(value)
        for dependency in dependencies:
            if dependency.startswith(allowed_os):
                continue  # OS ABI libraries, never a system Python installation.
            if dependency.startswith("@rpath/"):
                candidates = [expand(value) / dependency.removeprefix("@rpath/") for value in rpaths]
                # The interpreter's LC_RPATH is inherited by extension modules.
                candidates.append(root / "python/lib" / dependency.removeprefix("@rpath/"))
            else:
                candidates = [expand(dependency)]
            require(any(inside(path, root).is_file() for path in candidates),
                    f"Unresolved/non-bundled native dependency: {binary}: {dependency}")
    require(checked > 0, "No Mach-O interpreter found")
    return checked, ignored_rpaths


def verify(root):
    require(sys.version_info[:3] == (3, 12, 12), "Incorrect managed Python version")
    inside(Path(sys.executable), root)
    inside(Path(sys.prefix), root)
    inside(Path(sys.base_prefix), root)
    for path in sys.path:
        require(bool(path), "Ambient working directory on import path")
        inside(Path(path), root)
    # This is an import-location proof, not an SDK startup/test. find_spec on
    # top-level names does not execute Hermes, load profiles, or make requests.
    sys.path[:0] = [str(root / "controller"), str(root / "source")]
    modules = ["run_agent", "hermes_cli", "agent", "forma_runtime", "openai", "httpx", "pydantic",
               "yaml", "cryptography", "ssl", "sqlite3", "ctypes"]
    for name in modules:
        spec = importlib.util.find_spec(name)
        require(spec is not None, f"Missing bundled module: {name}")
        if spec.origin not in (None, "built-in", "frozen"):
            inside(Path(spec.origin), root)
        for path in spec.submodule_search_locations or []:
            inside(Path(path), root)
    # Exercise CPython's native stdlib, not Hermes or any provider package.
    import ssl
    import sqlite3
    import ctypes
    require(ssl.OPENSSL_VERSION and sqlite3.sqlite_version and ctypes.sizeof(ctypes.c_void_p) == 8,
            "Bundled native stdlib unavailable")
    binaries, inert_rpaths = macho_dependencies(root)
    print(json.dumps({"python": sys.version.split()[0], "platform": sys.platform,
                      "executableInsideBundle": True, "importPathsInsideBundle": True,
                      "nativeBinariesChecked": binaries, "inertUpstreamRpaths": inert_rpaths, "hermesExecuted": False}))


if __name__ == "__main__":
    action, directory = sys.argv[1:]
    root = Path(directory).resolve(strict=True)
    if action == "finalize":
        finalize(root)
    elif action == "verify":
        verify(root)
    else:
        raise SystemExit("Unknown bundle operation")
