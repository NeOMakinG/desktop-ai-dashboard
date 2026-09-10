"""Fail-closed platform containment. No provider or Hermes imports here."""
from __future__ import annotations

import ctypes
import json
import os
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

from .contracts import Fault, require


def sb_string(path):
    # SBPL strings use the same quote/backslash escapes for these path scalars.
    value = str(Path(path).resolve())
    require(not any(ord(c) < 32 or 0xD800 <= ord(c) <= 0xDFFF for c in value),
            "Invalid sandbox path encoding", "sandbox_path_unsupported", 503)
    return json.dumps(value, ensure_ascii=False)


def mac_paths(config, relay):
    source = config.source.resolve()
    venv = config.python.absolute().parent.parent.resolve()
    python_root = config.python.resolve().parent.parent
    runtime = Path(__file__).resolve().parent.parent
    home = relay.parent.resolve() / "home"
    if config.managed:
        resource_root = runtime.parent
        require(runtime.name == "controller" and source == resource_root / "source"
                and venv == resource_root / "python" and python_root == venv,
                "Managed worker resources must share the verified standalone bundle", "sandbox_path_unsupported", 503)
    # Never permit a profile/home-wide read mount, even if native configuration
    # accidentally names the wrong interpreter or SDK directory.
    for root in (source, venv, python_root, runtime):
        require(root not in (Path("/"), Path("/Users"), Path.home(), Path("/Library"), Path("/opt"), Path("/usr")),
                "Sandbox read root too broad", "sandbox_path_unsupported", 503)
    return source, venv, python_root, runtime, home


def mac_profile(config, relay):
    source, venv, python_root, runtime, home = mac_paths(config, relay)
    reads = [source, venv, python_root, runtime, Path("/System/Library"), Path("/usr/lib"),
             Path("/Library/Apple/System/Library")]
    # kern.procargs2 needs an explicit process-info deny: deny-default and
    # filtered sysctl-read alone do NOT block sibling argv/environment on macOS.
    # Keep only hardware/version sysctls and explicitly re-allow self metadata.
    sysctls = ("hw.memsize", "hw.ncpu", "hw.availcpu", "hw.activecpu", "hw.logicalcpu", "hw.logicalcpu_max",
               "hw.physicalcpu", "hw.physicalcpu_max", "hw.pagesize", "hw.cputype", "hw.cpusubtype", "hw.cpufamily",
               "hw.machine", "hw.model", "hw.cachelinesize", "hw.tbfrequency", "hw.optional.arm64", "hw.optional.neon",
               "kern.osrelease", "kern.osversion", "kern.ostype", "kern.version", "kern.osproductversion", "kern.hostname",
               "kern.argmax", "sysctl.proc_translated")
    sysctl_rule = "(allow sysctl-read " + " ".join("(sysctl-name " + json.dumps(name) + ")" for name in sysctls) + ")"
    lines = ["(version 1)", "(deny default)", "(deny process-info*)", sysctl_rule,
             "(allow process-info* (target self))", "(allow signal (target self))",
             "(allow file-read-metadata)",
             '(allow file-read* (literal "/") (literal "/usr/bin/env") (literal "/dev/null") (literal "/dev/urandom") (literal "/dev/random"))',
             '(allow file-write-data (literal "/dev/null"))',
             '(allow process-exec (literal "/usr/bin/env") (literal ' + sb_string(config.python.resolve()) + '))']
    lines += ["(allow file-read* (subpath " + sb_string(path) + "))" for path in reads]
    lines += ["(allow file-read* file-write* (subpath " + sb_string(home) + "))",
              "(allow network-outbound (remote unix-socket (path-literal " + sb_string(relay) + ")))"]
    return "\n".join(lines)


def mac_command(config, relay):
    require(sys.platform == "darwin" and Path("/usr/bin/sandbox-exec").is_file(),
            "macOS sandbox-exec unavailable", "sandbox_unavailable", 503)
    source, _, _, runtime, home = mac_paths(config, relay)
    (home / "hermes").mkdir(parents=True, exist_ok=True, mode=0o700)
    (home / "tmp").mkdir(exist_ok=True, mode=0o700)
    return ["/usr/bin/sandbox-exec", "-p", mac_profile(config, relay), "/usr/bin/env", "-i",
            "HOME=" + str(home), "HERMES_HOME=" + str(home / "hermes"), "TMPDIR=" + str(home / "tmp"),
            "HERMES_SAFE_MODE=1", "FORMA_HERMES_SOURCE=" + str(source), "FORMA_RELAY_SOCKET=" + str(relay.resolve()),
            "FORMA_WORKER_HOME=" + str(home), "PYTHONPATH=" + str(runtime), "PYTHONNOUSERSITE=1",
            "PYTHONDONTWRITEBYTECODE=1", "PYTHONUNBUFFERED=1", "LANG=en_US.UTF-8", "PATH=/usr/bin:/bin",
            str(config.python), "-B", "-m", "forma_runtime.worker"]


def mac_probe(config):
    """Verify exact production profile with stdlib probes, never a model request.

    Read/write, ambient env, IP sockets and an unapproved Unix socket must fail;
    only the fresh HOME and single explicitly mounted Unix relay can communicate.
    Any unsupported SBPL primitive or Python layout leaves readiness false.
    """
    with tempfile.TemporaryDirectory(prefix="forma-sandbox-probe-") as directory:
        root = Path(directory).resolve()
        sentinel = root / "host-private"; sentinel.write_text("synthetic-isolation-sentinel")
        allowed = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        denied = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        tcp = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        canary = None
        try:
            # The ONLY process inspected is this owned nonsecret sibling. Never
            # inspect a real application's argument/environment vector.
            canary = subprocess.Popen([str(config.python), "-I", "-B", "-c", "import time;time.sleep(20)",
                "FORMA_NONSECRET_ARG_CANARY"], env={"PATH": "/usr/bin:/bin", "FORMA_NONSECRET_ENV_CANARY": "synthetic-only"},
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            relay = root / "gateway.sock"
            allowed.bind(str(relay)); allowed.listen(1)
            forbidden = root / "denied.sock"; denied.bind(str(forbidden)); denied.listen(1)
            tcp.bind(("127.0.0.1", 0)); tcp.listen(1)
            script = r'''
import ctypes,json,os,pathlib,socket
out={}
def attempt(name, fn):
 try: fn();out[name]=True
 except (OSError,PermissionError):out[name]=False
def connect(family, address):
 with socket.socket(family,socket.SOCK_STREAM) as s:s.settimeout(.4);s.connect(address)
def writable(path):
 fd=os.open(path,os.O_WRONLY);os.close(fd)
def fork():
 pid=os.fork()
 if pid==0:os._exit(0)
 os.waitpid(pid,0)
args=json.loads(__import__('sys').argv[1])
attempt('hostRead',lambda:pathlib.Path(args['sentinel']).read_bytes())
attempt('sourceWrite',lambda:writable(args['source']))
attempt('pythonWrite',lambda:writable(args['python']))
attempt('ip',lambda:connect(socket.AF_INET,('127.0.0.1',args['port'])))
attempt('unixDenied',lambda:connect(socket.AF_UNIX,args['denied']))
attempt('unixAllowed',lambda:connect(socket.AF_UNIX,os.environ['FORMA_RELAY_SOCKET']))
attempt('homeWrite',lambda:(pathlib.Path(os.environ['HOME'])/'probe').write_text('synthetic'))
attempt('fork',fork)
out['envSecret']='FORMA_PROBE_PARENT_SECRET' in os.environ
libc=ctypes.CDLL(None,use_errno=True)
mib=(ctypes.c_int*3)(1,49,args['canaryPid'])
data=ctypes.create_string_buffer(131072);size=ctypes.c_size_t(len(data))
out['processArgs']=libc.sysctl(mib,3,data,ctypes.byref(size),None,0)==0
print(json.dumps(out))
'''
            command = mac_command(config, relay)
            command[-2:] = ["-c", script, json.dumps({"sentinel": str(sentinel), "source": str(config.source / "run_agent.py"),
                "python": str(config.python.resolve()), "port": tcp.getsockname()[1], "denied": str(forbidden), "canaryPid": canary.pid})]
            result = subprocess.run(command, capture_output=True, timeout=10,
                                    env={"PATH": "/usr/bin:/bin", "FORMA_PROBE_PARENT_SECRET": "synthetic-only"}, cwd=root / "home")
            require(result.returncode == 0 and canary.poll() is None, "macOS sandbox probe failed", "sandbox_verification_failed", 503)
            require(json.loads(result.stdout) == {"hostRead": False, "sourceWrite": False, "pythonWrite": False,
                    "ip": False, "unixDenied": False, "unixAllowed": True, "homeWrite": True, "fork": False, "envSecret": False, "processArgs": False},
                    "macOS containment probe failed", "sandbox_verification_failed", 503)
        finally:
            if canary:
                canary.terminate()
                try: canary.wait(timeout=2)
                except subprocess.TimeoutExpired: canary.kill(); canary.wait(timeout=2)
            allowed.close(); denied.close(); tcp.close()


class MacMemoryGuard:
    """Parent-side resident memory ceiling; Darwin does not enforce RLIMIT_AS."""
    def __init__(self, pid):
        self.pid = pid
        self.library = ctypes.CDLL("/usr/lib/libproc.dylib", use_errno=True)
        self.query = self.library.proc_pid_rusage
        self.query.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_void_p]
        self.query.restype = ctypes.c_int

    def check(self):
        # rusage_info_v0: 16 byte UUID, then user/system/wakeup/pagein/wired/rss.
        class Usage(ctypes.Structure):
            _fields_ = [("uuid", ctypes.c_byte * 16), ("values", ctypes.c_uint64 * 10)]
        usage = Usage()
        require(self.query(self.pid, 0, ctypes.byref(usage)) == 0,
                "Worker memory accounting unavailable", "resource_accounting_failed", 503)
        require(usage.values[6] <= 2_147_483_648, "Worker resident memory exceeded", "budget_exceeded", 429)
