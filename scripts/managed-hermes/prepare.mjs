import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import * as fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../..');
const cache = path.join(repo, '.local/managed-hermes');
const output = path.join(repo, 'src-tauri/resources/managed-hermes');
const pins = JSON.parse(fs.readFileSync(path.join(here, 'pins.json'), 'utf8'));
const platform = `${process.platform}-${process.arch}`;
const target = pins.platforms[platform];
const sha = data => createHash('sha256').update(data).digest('hex');
const hashFile = file => sha(fs.readFileSync(file));
const json = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n');
const resolveInside = (root, relative) => {
  assert(!path.isAbsolute(relative) && !relative.split(/[\\/]/).includes('..'), `Unsafe resource path: ${relative}`);
  const result = path.resolve(root, relative);
  assert(result.startsWith(path.resolve(root) + path.sep), `Resource escaped root: ${relative}`);
  return result;
};

function files(root) {
  const result = [];
  for (const item of fs.readdirSync(root, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name, 'en'))) {
    const file = path.join(root, item.name);
    assert(!item.isSymbolicLink(), `Unexpected symlink: ${file}`);
    if (item.isDirectory()) result.push(...files(file));
    else { assert(item.isFile(), `Unsupported resource type: ${file}`); result.push(file); }
  }
  return result;
}

function treeHash(root, pythonOnly = false) {
  return sha(files(root).filter(file => !pythonOnly || file.endsWith('.py'))
    .map(file => `${path.relative(root, file)}:${hashFile(file)}`).join('\n'));
}

const controllerSource = path.join(repo, 'runtime/forma_runtime');
function inputHash() { return sha(`${treeHash(here)}:${treeHash(controllerSource, true)}`); }

function copyController(destination, expected) {
  fs.cpSync(controllerSource, destination, { recursive: true,
    filter: file => { const stat = fs.lstatSync(file); assert(!stat.isSymbolicLink(), `Controller symlink: ${file}`);
      return stat.isDirectory() ? path.basename(file) !== '__pycache__' : file.endsWith('.py'); } });
  assert.equal(treeHash(destination, true), expected, 'Controller changed while copying; retry after source edits finish');
  assert.equal(treeHash(controllerSource, true), expected, 'Controller source changed during preparation');
}

function publish(stage, work, destination = output) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  const old = path.join(work, 'previous-bundle');
  if (fs.existsSync(destination)) fs.renameSync(destination, old);
  try { fs.renameSync(stage, destination); }
  catch (error) { if (fs.existsSync(old)) fs.renameSync(old, destination); throw error; }
}

function seal(root, fingerprint) {
  assert.equal(inputHash(), fingerprint, 'Build/controller sources changed during preparation; retry after edits finish');
  fs.rmSync(path.join(root, 'checksums.json'), { force: true });
  json(path.join(root, 'checksums.json'), Object.fromEntries(files(root).map(file => [
    path.relative(root, file).split(path.sep).join('/'), hashFile(file)])));
  verify(root, fingerprint);
}

function environment() {
  return { HOME: path.join(cache, 'build-home'), TMPDIR: path.join(cache, 'tmp'), PATH: '/usr/bin:/bin',
    LANG: 'en_US.UTF-8', UV_CACHE_DIR: path.join(cache, 'uv-cache'), UV_PYTHON_DOWNLOADS: 'never',
    UV_NO_PROGRESS: '1', UV_CONCURRENT_DOWNLOADS: '1', UV_CONCURRENT_INSTALLS: '1', UV_CONCURRENT_BUILDS: '1',
    PIP_CONFIG_FILE: '/dev/null', PIP_DISABLE_PIP_VERSION_CHECK: '1', PYTHONDONTWRITEBYTECODE: '1' };
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: cache, env: environment(), encoding: 'utf8',
    maxBuffer: 32 * 1024 * 1024, timeout: 300_000, ...options });
  if (result.status !== 0 || result.error) {
    throw new Error(`${path.basename(command)} failed (${result.status}): ${result.error?.message ?? ''}\n${result.stderr ?? ''}\n${result.stdout ?? ''}`);
  }
  return result.stdout ?? '';
}

function guard() {
  const space = fs.statfsSync(repo);
  const load = os.loadavg()[0];
  const pressure = spawnSync('/usr/bin/memory_pressure', [], { encoding: 'utf8', timeout: 10_000 }).stdout ?? '';
  const match = pressure.match(/System-wide memory free percentage: (\d+)%/);
  const memoryFree = Number(match?.[1]);
  const top = spawnSync('/usr/bin/top', ['-l', '2', '-s', '1', '-n', '0'], { encoding: 'utf8', timeout: 10_000 }).stdout ?? '';
  const samples = [...top.matchAll(/CPU usage: [0-9.]+% user, [0-9.]+% sys, ([0-9.]+)% idle/g)];
  const idle = Number(samples.at(-1)?.[1]);
  console.log(`Managed Hermes guard: ${platform}, load ${load.toFixed(1)}, CPU idle ${idle}%, effective memory free ${memoryFree}%, disk ${(space.bavail * space.bsize / 1024 ** 3).toFixed(1)} GiB`);
  const processTable = spawnSync('/bin/ps', ['-Ao', 'pid,pcpu,comm', '-r'], { encoding: 'utf8', timeout: 10_000 }).stdout ?? '';
  const busiest = processTable.split('\n').slice(1, 9).map(line => {
    const fields = line.match(/^\s*(\d+)\s+[\d.]+\s+(.+)$/);
    return fields ? { pid: Number(fields[1]), process: path.basename(fields[2]) } : null;
  }).filter(Boolean);
  console.log(`Top CPU processes (names/PIDs only): ${JSON.stringify(busiest)}`);
  assert(space.bavail * space.bsize >= 2 * 1024 ** 3, 'Managed Hermes preparation needs at least 2 GiB free disk');
  assert(load < os.availableParallelism() * 2, 'Host busy; queue managed Hermes preparation until CPU load settles');
  assert(Number.isFinite(memoryFree) && memoryFree >= 15, 'Host memory pressure too high or unavailable; queue asset preparation');
  assert(Number.isFinite(idle) && idle >= 15, 'Host CPU idle too low or unavailable; queue asset preparation');
}

function download(asset, name) {
  assert(asset.url.startsWith('https://'), 'Only pinned HTTPS assets are supported');
  const destination = path.join(cache, 'downloads', name);
  if (fs.existsSync(destination)) {
    assert.equal(hashFile(destination), asset.sha256, `Cached asset checksum mismatch: ${name}`);
    return destination;
  }
  console.log(`Downloading pinned ${name}`);
  const temporary = destination + '.partial';
  try {
    run('/usr/bin/curl', ['--fail', '--location', '--silent', '--show-error', '--proto', '=https', '--proto-redir', '=https',
      '--connect-timeout', '20', '--max-time', '240', '--retry', '2', '--output', temporary, asset.url]);
    assert.equal(hashFile(temporary), asset.sha256, `Download checksum mismatch: ${name}`);
    fs.renameSync(temporary, destination);
  } finally {
    fs.rmSync(temporary, { force: true });
  }
  return destination;
}

function extract(archive, directory, members = []) {
  // Even checksum-pinned archives cannot introduce absolute/parent paths.
  for (const entry of run('/usr/bin/tar', ['-tf', archive]).trim().split('\n')) {
    assert(!entry.startsWith('/') && !entry.split('/').includes('..'), `Unsafe archive entry: ${entry}`);
  }
  fs.mkdirSync(directory, { recursive: true });
  run('/usr/bin/tar', ['-xf', archive, '-C', directory, ...members]);
}

function copySource(source, destination) {
  fs.mkdirSync(destination, { recursive: true });
  // The SDK's declared Python package roots, supporting data, and original
  // metadata. Every shipped byte remains identical to the public source pin.
  const packages = new Set(['acp_adapter', 'agent', 'tools', 'hermes_cli', 'gateway', 'tui_gateway', 'cron',
    'plugins', 'providers', 'assets', 'locales']);
  const metadata = new Set(['LICENSE', 'README.md', 'pyproject.toml', 'uv.lock']);
  const allowed = item => !item.startsWith('.env') && !['credentials.json', '__pycache__'].includes(item)
    && !/\.(pem|key|pyc|pyo)$/i.test(item);
  for (const item of fs.readdirSync(source, { withFileTypes: true })) {
    if (!(packages.has(item.name) || metadata.has(item.name) || (item.isFile() && item.name.endsWith('.py')))) continue;
    fs.cpSync(path.join(source, item.name), path.join(destination, item.name), { recursive: true,
      filter: file => { assert(!fs.lstatSync(file).isSymbolicLink(), `Symlink in public source: ${file}`); return allowed(path.basename(file)); } });
  }
}

function verify(root, expectedInputs = inputHash()) {
  assert(fs.lstatSync(root).isDirectory() && !fs.lstatSync(root).isSymbolicLink(), 'Managed resource root must be an owned directory');
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8'));
  assert.equal(manifest.schemaVersion, 1);
  assert.equal(manifest.inputSha256, expectedInputs, 'Managed resources are stale; run pnpm runtime:prepare');
  assert.equal(manifest.platform, platform, 'Managed resources target another platform');
  assert.equal(manifest.hermes.commit, pins.hermes.commit);
  assert.equal(manifest.hermes.version, pins.hermes.version);
  assert.equal(manifest.python.executable, target.python.executable);
  const checksums = JSON.parse(fs.readFileSync(path.join(root, 'checksums.json'), 'utf8'));
  const actual = files(root).map(file => path.relative(root, file).split(path.sep).join('/')).filter(file => file !== 'checksums.json').sort();
  assert.deepEqual(actual, Object.keys(checksums).sort(), 'Missing or unexpected managed resource files');
  for (const [relative, expected] of Object.entries(checksums)) {
    assert.match(expected, /^[0-9a-f]{64}$/);
    assert.equal(hashFile(resolveInside(root, relative)), expected, `Managed resource checksum mismatch: ${relative}`);
  }
  assert.equal(hashFile(path.join(root, 'source/uv.lock')), pins.hermes.lockSha256);
  assert.equal(hashFile(path.join(root, 'source/pyproject.toml')), pins.hermes.projectSha256);
  assert.equal(hashFile(path.join(root, 'source/LICENSE')), pins.hermes.licenseSha256);
  assert(fs.statSync(path.join(root, target.python.executable)).mode & 0o111, 'Managed Python is not executable');
  assert(fs.existsSync(path.join(root, 'controller/forma_runtime/managed.py')), 'Managed controller entrypoint missing');
  return manifest;
}

// The Scrapling engine's Chromium (headless shell) lives in its own sealed
// resource root so the Hermes core manifest, Mach-O walk, and controller
// refresh semantics stay untouched. The tree is symlink-free by choice of
// the headless-shell build; any symlink fails verification.
const browsersOutput = path.join(repo, 'src-tauri/resources/managed-browsers');

function verifyBrowsers(root = browsersOutput, fingerprint = treeHash(here)) {
  assert(fs.lstatSync(root).isDirectory() && !fs.lstatSync(root).isSymbolicLink(), 'Browsers resource root must be an owned directory');
  const chromium = target.browserEngine.chromium;
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'manifest.json'), 'utf8'));
  assert.equal(manifest.schemaVersion, 1);
  assert.equal(manifest.platform, platform);
  assert.equal(manifest.kind, 'chromium-headless-shell');
  assert.equal(manifest.inputSha256, fingerprint, 'Managed browsers are stale; run pnpm runtime:prepare');
  assert.equal(manifest.chromium.revision, chromium.revision);
  assert.equal(manifest.chromium.browserVersion, chromium.browserVersion);
  assert.equal(manifest.chromium.executable, chromium.executable);
  const checksums = JSON.parse(fs.readFileSync(path.join(root, 'checksums.json'), 'utf8'));
  const actual = files(root).map(file => path.relative(root, file).split(path.sep).join('/')).filter(file => file !== 'checksums.json').sort();
  assert.deepEqual(actual, Object.keys(checksums).sort(), 'Missing or unexpected managed browser files');
  for (const [relative, expected] of Object.entries(checksums)) {
    assert.match(expected, /^[0-9a-f]{64}$/);
    assert.equal(hashFile(resolveInside(root, relative)), expected, `Managed browser checksum mismatch: ${relative}`);
  }
  assert(fs.statSync(resolveInside(root, chromium.executable)).mode & 0o111, 'Managed Chromium is not executable');
  return manifest;
}

function ensureBrowsers() {
  if (fs.existsSync(path.join(browsersOutput, 'manifest.json'))) {
    try {
      verifyBrowsers();
      console.log('Managed browsers already verified; no network/download.');
      return;
    } catch { /* rebuild below */ }
  }
  guard();
  const chromium = target.browserEngine.chromium;
  const work = fs.mkdtempSync(path.join(cache, 'browsers-'));
  const stage = path.join(work, 'bundle');
  try {
    const archive = download(chromium, `chromium-${chromium.revision}.zip`);
    fs.mkdirSync(path.join(stage, chromium.directory), { recursive: true });
    extract(archive, path.join(stage, chromium.directory));
    const unpacked = path.join(stage, chromium.directory, chromium.archiveRoot);
    assert(fs.existsSync(path.join(unpacked, 'chrome-headless-shell')), 'Headless shell missing from pinned archive');
    fs.mkdirSync(path.join(stage, 'licenses'));
    fs.copyFileSync(path.join(unpacked, 'LICENSE.headless_shell'), path.join(stage, chromium.license));
    json(path.join(stage, 'manifest.json'), { schemaVersion: 1, platform, kind: 'chromium-headless-shell',
      inputSha256: treeHash(here), archiveSha256: chromium.sha256,
      chromium: { revision: chromium.revision, browserVersion: chromium.browserVersion, executable: chromium.executable } });
    json(path.join(stage, 'checksums.json'), Object.fromEntries(files(stage)
      .filter(file => path.relative(stage, file) !== 'checksums.json')
      .map(file => [path.relative(stage, file).split(path.sep).join('/'), hashFile(file)])));
    verifyBrowsers(stage);
    publish(stage, work, browsersOutput);
    console.log(`Managed browsers ${chromium.browserVersion} prepared at ${browsersOutput}`);
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

function refreshController(previous, fingerprint, controllerFingerprint) {
  guard();
  const work = fs.mkdtempSync(path.join(cache, 'refresh-'));
  const stage = path.join(work, 'bundle');
  try {
    verify(output, previous.inputSha256);
    fs.cpSync(output, stage, { recursive: true, mode: fs.constants.COPYFILE_FICLONE });
    fs.rmSync(path.join(stage, 'controller'), { recursive: true });
    copyController(path.join(stage, 'controller/forma_runtime'), controllerFingerprint);
    json(path.join(stage, 'manifest.json'), { ...previous, inputSha256: fingerprint,
      controllerInputSha256: controllerFingerprint });
    const receipt = run(path.join(stage, target.python.executable), ['-I', '-B', path.join(here, 'bundle.py'), 'verify', stage]);
    json(path.join(stage, 'relocation-check.json'), JSON.parse(receipt));
    seal(stage, fingerprint);
    publish(stage, work);
    console.log('Managed controller refreshed and relocation/inventory verified; no SDK reinstall or downloads.');
  } finally { fs.rmSync(work, { recursive: true, force: true }); }
}

function prepare() {
  const sdkFingerprint = treeHash(here);
  const controllerFingerprint = treeHash(controllerSource, true);
  const fingerprint = sha(`${sdkFingerprint}:${controllerFingerprint}`);
  ensureBrowsers();
  if (fs.existsSync(path.join(output, 'manifest.json'))) {
    const previous = JSON.parse(fs.readFileSync(path.join(output, 'manifest.json'), 'utf8'));
    if (previous.inputSha256 === fingerprint && previous.platform === platform && !process.argv.includes('--refresh-controller')) {
      verify(output, fingerprint);
      console.log('Managed Hermes resources already verified; no network/download.');
      return;
    }
    if (previous.sdkInputSha256 === sdkFingerprint && previous.platform === platform) {
      refreshController(previous, fingerprint, controllerFingerprint);
      return;
    }
  }
  guard();
  const sourceArchive = download(pins.hermes, 'hermes-349e6611.tar.gz');
  const pythonArchive = download(target.python, 'python.tar.gz');
  const licenseArchive = download(target.python.licenses, 'python-full.tar.zst');
  const exporterArchive = download(target.exporter, 'uv-0.9.26.tar.gz');
  const decoderWheel = download(target.archiveDecoder, 'zstandard-0.25.0-cp312-cp312-macosx_11_0_arm64.whl');
  const work = fs.mkdtempSync(path.join(cache, 'prepare-'));
  const stage = path.join(work, 'bundle');
  fs.mkdirSync(stage);
  try {
    extract(pythonArchive, stage);
    extract(sourceArchive, path.join(work, 'upstream'));
    const source = path.join(work, 'upstream', `hermes-agent-${pins.hermes.commit}`);
    assert.equal(hashFile(path.join(source, 'uv.lock')), pins.hermes.lockSha256);
    assert.equal(hashFile(path.join(source, 'pyproject.toml')), pins.hermes.projectSha256);
    assert.equal(hashFile(path.join(source, 'LICENSE')), pins.hermes.licenseSha256);
    copySource(source, path.join(stage, 'source'));
    extract(exporterArchive, path.join(work, 'exporter'));
    const exporter = path.join(work, 'exporter', target.exporter.executable);
    const requirements = path.join(stage, 'requirements-core.txt');
    run(exporter, ['--no-config', 'export', '--frozen', '--no-default-groups', '--no-dev', '--no-emit-project',
      '--no-header', '--project', source, '--output-file', requirements]);
    const interpreter = path.join(stage, target.python.executable);
    const pip = ['-I', '-B', '-m', 'pip', '--isolated', '--disable-pip-version-check'];
    const wheels = path.join(cache, 'wheels');
    run(interpreter, [...pip, 'download', '--only-binary=:all:', '--require-hashes', '--no-deps',
      '--index-url', 'https://pypi.org/simple', '--dest', wheels, '-r', requirements]);
    run(interpreter, [...pip, 'install', '--no-index', '--find-links', wheels, '--only-binary=:all:',
      '--require-hashes', '--no-deps', '--no-compile', '-r', requirements]);
    // The Scrapling browser engine rides the same pinned interpreter and the
    // same offline wheel discipline as the Hermes core set.
    const scraplingRequirements = path.join(here, pins.engines.scrapling.requirements);
    assert.equal(sha(fs.readFileSync(scraplingRequirements)), pins.engines.scrapling.requirementsSha256,
      'Scrapling requirements drifted from pins.json; re-pin consciously');
    const scraplingWheels = path.join(cache, 'wheels-scrapling');
    run(interpreter, [...pip, 'download', '--only-binary=:all:', '--require-hashes', '--no-deps',
      '--index-url', 'https://pypi.org/simple', '--dest', scraplingWheels, '-r', scraplingRequirements]);
    run(interpreter, [...pip, 'install', '--no-index', '--find-links', scraplingWheels, '--only-binary=:all:',
      '--require-hashes', '--no-deps', '--no-compile', '-r', scraplingRequirements]);
    run(interpreter, [...pip, 'check']);
    json(path.join(stage, 'wheel-checksums.json'), Object.fromEntries(files(wheels).filter(file => file.endsWith('.whl'))
      .map(file => [path.basename(file), hashFile(file)])));
    json(path.join(stage, 'wheel-checksums-scrapling.json'), Object.fromEntries(files(scraplingWheels).filter(file => file.endsWith('.whl'))
      .map(file => [path.basename(file), hashFile(file)])));
    // macOS bsdtar delegates zstd to PATH. Use a hash-pinned build-only wheel
    // instead; neither Homebrew nor the decoder becomes a runtime dependency.
    const decoder = path.join(work, 'archive-decoder');
    run(interpreter, ['-I', '-B', '-m', 'zipfile', '-e', decoderWheel, decoder]);
    const licenseTar = path.join(work, 'python-full.tar');
    run(interpreter, ['-I', '-B', '-c',
      'import sys; sys.path.insert(0, sys.argv[1]); import zstandard; ' +
      'zstandard.ZstdDecompressor().copy_stream(open(sys.argv[2], "rb"), open(sys.argv[3], "wb"))',
      decoder, licenseArchive, licenseTar]);
    extract(licenseTar, path.join(work, 'python-licenses'), ['python/licenses', 'python/PYTHON.json']);
    fs.mkdirSync(path.join(stage, 'licenses'));
    fs.cpSync(path.join(work, 'python-licenses/python/licenses'), path.join(stage, 'licenses/python'), { recursive: true });
    fs.copyFileSync(path.join(work, 'python-licenses/python/PYTHON.json'), path.join(stage, 'licenses/python-build.json'));
    copyController(path.join(stage, 'controller/forma_runtime'), controllerFingerprint);
    json(path.join(stage, 'build-pins.json'), pins);
    run(interpreter, ['-I', '-B', path.join(here, 'bundle.py'), 'finalize', stage]);
    // Actually relocate before invoking Python. A venv or absolute libpython
    // reference cannot pass by continuing to use its original build location.
    const relocated = path.join(work, 'relocated bundle with spaces');
    fs.renameSync(stage, relocated);
    const receipt = run(path.join(relocated, target.python.executable), ['-I', '-B', path.join(here, 'bundle.py'), 'verify', relocated]);
    console.log(receipt.trim());
    json(path.join(relocated, 'relocation-check.json'), JSON.parse(receipt));
    json(path.join(relocated, 'manifest.json'), { schemaVersion: 1, platform, target: target.target,
      minimumOS: target.minimumOS, inputSha256: fingerprint,
      sdkInputSha256: sdkFingerprint, controllerInputSha256: controllerFingerprint,
      hermes: { ...pins.hermes, source: 'source', proof: 'source-proof.json' },
      python: { version: target.python.version, release: target.python.release, executable: target.python.executable },
      controller: { path: 'controller', entrypoint: 'controller/forma_runtime/managed.py' },
      dependencyPolicy: 'frozen-core-only-wheels', platformQA: 'asset-relocation-only; native app not launched' });
    seal(relocated, fingerprint);
    publish(relocated, work);
    console.log(`Managed Hermes ${pins.hermes.version} prepared at ${output}`);
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}

try {
  assert(target, `Managed Hermes is not packaged for ${platform}; no system Python fallback. Verified platform: darwin-arm64 only.`);
  const requested = process.env.TAURI_ENV_TARGET_TRIPLE || process.env.TARGET;
  assert(!requested || requested === target.target, `Cross-target bundle unavailable: ${requested}`);
  if (process.argv.includes('--verify')) {
    verify(output);
    verifyBrowsers();
    console.log('Managed Hermes resource inventory verified (offline).');
  } else {
    for (const dir of ['downloads', 'tmp', 'build-home', 'wheels', 'wheels-scrapling']) fs.mkdirSync(path.join(cache, dir), { recursive: true });
    const lock = path.join(cache, 'prepare.lock');
    let handle;
    try { handle = fs.openSync(lock, 'wx', 0o600); }
    catch { throw new Error('Managed Hermes asset preparation already owns the build slot (prepare.lock).'); }
    try { fs.writeFileSync(handle, `${process.pid}\n`); prepare(); }
    finally { fs.closeSync(handle); fs.unlinkSync(lock); }
  }
} catch (error) {
  console.error(`Managed Hermes packaging failed: ${error.message}`);
  process.exitCode = 1;
}
