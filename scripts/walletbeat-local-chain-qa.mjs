#!/usr/bin/env node
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import net from 'node:net';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');

const walletbeatRepo = 'https://github.com/walletbeat/walletbeat.git';
const walletbeatRef = process.env.DECKARD_WALLETBEAT_REF ?? '4062044982c9e2878d64d8f38c59fdc87e276554';
const walletbeatDir = path.join(repoRoot, '.walletbeat', 'walletbeat');
const extensionDir = path.join(repoRoot, 'extension');
const artifactsDir = path.join(repoRoot, 'test-results', 'walletbeat');
const profileDir = path.join(repoRoot, '.playwright', 'walletbeat-local-chain-profile');
const headed = process.env.DECKARD_WALLETBEAT_HEADED === '1';
const account = '0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266';
const expectedChainId = '0x7a69';
const chainIdDecimal = '31337';
const processes = [];
let qaDir;
let bridgePort;
let walletbeatPort;
let anvilPort;
let rpcUrl;
let bridgeUrl;

main().catch((error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error);
  process.exitCode = 1;
}).finally(async () => {
  await stopProcesses();
  if (qaDir) fs.rmSync(qaDir, { recursive: true, force: true });
});

async function main() {
  await requireCommand('anvil', ['--version']);
  fs.mkdirSync(artifactsDir, { recursive: true });
  fs.rmSync(profileDir, { recursive: true, force: true });
  validateExtension();
  await ensureWalletbeatCheckout();
  await run('pnpm', ['install', '--frozen-lockfile'], { cwd: walletbeatDir, name: 'pnpm' });

  bridgePort = Number(process.env.DECKARD_WALLETBEAT_BRIDGE_PORT ?? '8765');
  walletbeatPort = Number(process.env.DECKARD_WALLETBEAT_PORT ?? await freePort());
  anvilPort = Number(process.env.DECKARD_WALLETBEAT_ANVIL_PORT ?? await freePort());
  rpcUrl = `http://127.0.0.1:${anvilPort}`;
  bridgeUrl = `http://127.0.0.1:${bridgePort}/rpc`;
  qaDir = fs.mkdtempSync(path.join(os.tmpdir(), 'deckard-walletbeat-local-chain-'));
  const socketPath = path.join(qaDir, 'signerd.sock');

  await startAnvil();
  await run('cargo', ['run', '--quiet', '--locked', '-p', 'deckard-core', '--no-default-features', '--example', 'qa-vault'], {
    cwd: repoRoot,
    name: 'qa-vault',
    env: { ...process.env, DECKARD_CONFIG_DIR: qaDir },
    quiet: true,
  });
  await run('cargo', ['build', '--quiet', '--locked', '-p', 'deckard-signerd', '--no-default-features', '--example', 'qa-supervisor'], {
    cwd: repoRoot,
    name: 'qa-supervisor-build',
  });
  await startSupervisor(socketPath);
  await startBridge(socketPath);
  await startWalletbeat();

  const context = await chromium.launchPersistentContext(profileDir, {
    channel: 'chromium',
    headless: !headed,
    serviceWorkers: 'allow',
    args: [`--disable-extensions-except=${extensionDir}`, `--load-extension=${extensionDir}`],
  });

  try {
    const page = await context.newPage();
    const pageErrors = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') pageErrors.push(message.text());
    });

    await page.goto(`http://127.0.0.1:${walletbeatPort}/test/`, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => Boolean(window.ethereum?.request), { timeout: 15_000 });

    const results = await page.evaluate(async ({ account, chainId }) => {
      const provider = window.ethereum;
      if (!provider) throw new Error('window.ethereum missing');
      const accounts = await provider.request({ method: 'eth_requestAccounts' });
      const activeChain = await provider.request({ method: 'eth_chainId' });
      const nativeHash = await provider.request({
        method: 'eth_sendTransaction',
        params: [{ from: account, to: '0x0000000000000000000000000000000000000001', value: '0x1' }],
      });
      const transferHash = await provider.request({
        method: 'eth_sendTransaction',
        params: [{
          from: account,
          to: '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48',
          data: '0xa9059cbb00000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4240',
        }],
      });
      const approveHash = await provider.request({
        method: 'eth_sendTransaction',
        params: [{
          from: account,
          to: '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48',
          data: '0x095ea7b300000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4240',
        }],
      });
      const capabilities = await provider.request({
        method: 'wallet_getCapabilities',
        params: [account],
      });
      const batchResult = await provider.request({
        method: 'wallet_sendCalls',
        params: [{
          version: '2.0.0',
          chainId,
          from: account,
          atomicRequired: false,
          calls: [
            { to: '0x0000000000000000000000000000000000000002', value: '0x2' },
            {
              to: '0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48',
              data: '0xa9059cbb00000000000000000000000087870bca3f3fd6335c3f4ce8392d69350b4fa4e200000000000000000000000000000000000000000000000000000000000f4241',
            },
          ],
        }],
      });
      const batchId = typeof batchResult === 'string' ? batchResult : batchResult?.id;
      const batchStatus = await provider.request({
        method: 'wallet_getCallsStatus',
        params: [batchId],
      });
      const walletbeatProbeResult = await provider.request({
        method: 'wallet_sendCalls',
        params: [{
          version: '2.0.0',
          chainId,
          from: account,
          atomicRequired: false,
          calls: [{ to: '0x0000000000000000000000000000000000000000', value: '0x0', data: '0x00' }],
        }],
      });
      let atomicError = null;
      try {
        await provider.request({
          method: 'wallet_sendCalls',
          params: [{
            version: '2.0.0',
            chainId,
            from: account,
            atomicRequired: true,
            calls: [{ to: '0x0000000000000000000000000000000000000001', value: '0x1' }],
          }],
        });
      } catch (error) {
        atomicError = {
          code: typeof error === 'object' && error ? error.code : undefined,
          message: error instanceof Error ? error.message : String(error),
        };
      }
      const simpleSignature = await provider.request({
        method: 'personal_sign',
        params: ['0x68656c6c6f2066726f6d206c6f63616c20636861696e', account],
      });
      const siweSignature = await provider.request({
        method: 'personal_sign',
        params: [`localhost wants you to sign in with your Ethereum account:\n${account}\n\nDeckard WalletBeat local-chain QA.\n\nURI: http://127.0.0.1\nVersion: 1\nChain ID: ${Number(chainId)}\nNonce: deckardqa\nIssued At: 2026-06-24T00:00:00.000Z`, account],
      });
      const typedSignature = await provider.request({
        method: 'eth_signTypedData_v4',
        params: [
          account,
          {
            domain: {
              name: 'Deckard WalletBeat Local Chain QA',
              version: '1',
              chainId: Number(chainId),
              verifyingContract: '0x0000000000000000000000000000000000000000',
            },
            types: {
              EIP712Domain: [
                { name: 'name', type: 'string' },
                { name: 'version', type: 'string' },
                { name: 'chainId', type: 'uint256' },
                { name: 'verifyingContract', type: 'address' },
              ],
              TestMessage: [
                { name: 'purpose', type: 'string' },
                { name: 'message', type: 'string' },
              ],
            },
            primaryType: 'TestMessage',
            message: {
              purpose: 'Local-chain QA only',
              message: 'This signature is produced by a throwaway Deckard test vault.',
            },
          },
        ],
      });
      let ethSignError = null;
      try {
        await provider.request({ method: 'eth_sign', params: [account, '0x1234'] });
      } catch (error) {
        ethSignError = {
          code: typeof error === 'object' && error ? error.code : undefined,
          message: error instanceof Error ? error.message : String(error),
        };
      }
      return {
        accounts,
        activeChain,
        nativeHash,
        transferHash,
        approveHash,
        capabilities,
        batchId,
        batchStatus,
        walletbeatProbeResult,
        atomicError,
        simpleSignature,
        siweSignature,
        typedSignature,
        ethSignError,
      };
    }, { account, chainId: expectedChainId });

    const checks = [
      check('real daemon account', Array.isArray(results.accounts) && results.accounts[0] === account, results.accounts),
      check('local chain id', results.activeChain === expectedChainId, results.activeChain),
      check('native eth_sendTransaction via signerd', txHash(results.nativeHash), results.nativeHash),
      check('ERC-20 transfer(address,uint256) via signerd', txHash(results.transferHash), results.transferHash),
      check('ERC-20 approve(address,uint256) via signerd', txHash(results.approveHash), results.approveHash),
      check('wallet_getCapabilities EIP-5792 v2.0.0', Array.isArray(results.capabilities?.[expectedChainId]?.wallet_sendCalls?.supportedVersions) && results.capabilities[expectedChainId].wallet_sendCalls.supportedVersions.includes('2.0.0'), results.capabilities?.[expectedChainId]),
      check('wallet_sendCalls clear-signable non-atomic batch', typeof results.batchId === 'string' && results.batchId.startsWith('0x'), results.batchId),
      check('wallet_getCallsStatus for batch', results.batchStatus?.status === 200 && results.batchStatus?.atomic === false && Array.isArray(results.batchStatus?.receipts), results.batchStatus),
      check('wallet_sendCalls WalletBeat zero-value probe', (typeof results.walletbeatProbeResult === 'string' && results.walletbeatProbeResult.startsWith('0x')) || (typeof results.walletbeatProbeResult?.id === 'string' && results.walletbeatProbeResult.id.startsWith('0x')), results.walletbeatProbeResult),
      check('wallet_sendCalls atomicRequired refused', results.atomicError?.code === 4200 && /atomicRequired/.test(results.atomicError?.message ?? ''), results.atomicError),
      check('personal_sign via signerd', signature(results.simpleSignature), '<signature redacted>'),
      check('SIWE personal_sign via signerd', signature(results.siweSignature), '<signature redacted>'),
      check('EIP-712 typed data via signerd', signature(results.typedSignature), '<signature redacted>'),
      check('raw eth_sign refused', results.ethSignError?.code === 4200 && /raw eth_sign/.test(results.ethSignError?.message ?? ''), results.ethSignError),
    ];

    await page.screenshot({ path: path.join(artifactsDir, 'walletbeat-local-chain.png'), fullPage: true });
    const relevantPageErrors = pageErrors.filter((message) => !isIgnoredWalletbeatDevError(message));
    const report = {
      walletbeatRepo,
      walletbeatRef,
      mode: 'local-chain-real-signerd',
      url: `http://127.0.0.1:${walletbeatPort}/test/`,
      account,
      chainId: expectedChainId,
      checks,
      pageErrors: relevantPageErrors,
      ignoredPageErrors: pageErrors.filter(isIgnoredWalletbeatDevError),
    };
    fs.writeFileSync(path.join(artifactsDir, 'walletbeat-local-chain-results.json'), `${JSON.stringify(report, null, 2)}\n`);

    const failures = checks.filter((entry) => !entry.passed);
    if (failures.length > 0) throw new Error(`WalletBeat local-chain lane failed:\n${JSON.stringify(failures, null, 2)}`);
    if (relevantPageErrors.length > 0) throw new Error(`WalletBeat page emitted console/page errors:\n${relevantPageErrors.join('\n')}`);
    console.log(`WalletBeat local-chain lane passed:\n${checks.map((entry) => `${entry.name}: passed`).join('\n')}`);
  } finally {
    await context.close();
  }
}

async function ensureWalletbeatCheckout() {
  fs.mkdirSync(path.dirname(walletbeatDir), { recursive: true });
  if (!fs.existsSync(path.join(walletbeatDir, '.git'))) {
    await run('git', ['clone', '--filter=blob:none', walletbeatRepo, walletbeatDir], { cwd: repoRoot, name: 'git' });
  }
  await run('git', ['fetch', '--depth=1', 'origin', walletbeatRef], { cwd: walletbeatDir, name: 'git' });
  await run('git', ['checkout', '--detach', 'FETCH_HEAD'], { cwd: walletbeatDir, name: 'git' });
}

async function startAnvil() {
  const anvil = await spawnLogged(
    'anvil',
    ['--chain-id', chainIdDecimal, '--accounts', '10', '--balance', '10000', '--port', String(anvilPort)],
    { command: 'anvil', quiet: true },
  );
  processes.push(anvil);
  await waitForRpc();
}

async function startSupervisor(socketPath) {
  const supervisor = await spawnLogged(
    'qa-supervisor',
    [],
    {
      command: path.join(repoRoot, 'target', 'debug', 'examples', 'qa-supervisor'),
      env: {
        ...process.env,
        DECKARD_CONFIG_DIR: qaDir,
        DECKARD_SOCKET_PATH: socketPath,
        DECKARD_RPC_URL: rpcUrl,
        DECKARD_CHAIN_ID: chainIdDecimal,
      },
    },
  );
  processes.push(supervisor);
  await waitForFile(socketPath, 'signerd socket');
  await waitForLog(supervisor, 'qa-supervisor: ready', 'QA supervisor');
}

async function startBridge(socketPath) {
  const bridge = await spawnLogged(
    'bridge',
    ['run', '--quiet', '--locked', '-p', 'deckard-browser-bridge', '--no-default-features', '--', '--bind', `127.0.0.1:${bridgePort}`],
    {
      command: 'cargo',
      env: {
        ...process.env,
        DECKARD_CONFIG_DIR: qaDir,
        DECKARD_SOCKET_PATH: socketPath,
        DECKARD_RPC_URL: rpcUrl,
        DECKARD_CHAIN_ID: chainIdDecimal,
      },
    },
  );
  processes.push(bridge);
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (await bridgeReady()) return;
    await delay(250);
  }
  throw new Error(`Deckard browser bridge did not become ready on ${bridgeUrl}`);
}

async function startWalletbeat() {
  const walletbeat = await spawnLogged(
    'walletbeat',
    ['exec', 'astro', 'dev', '--host', '127.0.0.1', '--port', String(walletbeatPort)],
    {
      command: 'pnpm',
      cwd: walletbeatDir,
      env: { ...process.env, ASTRO_TELEMETRY_DISABLED: '1', WALLETBEAT_DEV: 'true' },
    },
  );
  processes.push(walletbeat);
  await waitForHttp(`http://127.0.0.1:${walletbeatPort}/test/`, 'WalletBeat dev server');
}

async function bridgeReady() {
  try {
    const response = await fetch(bridgeUrl, {
      method: 'POST',
      headers: { 'content-type': 'application/json', 'x-deckard-origin': `http://127.0.0.1:${walletbeatPort}` },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_chainId', params: [] }),
      signal: AbortSignal.timeout(1_000),
    });
    if (!response.ok) return false;
    const payload = await response.json();
    return payload.result === expectedChainId;
  } catch {
    return false;
  }
}

async function waitForRpc() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(rpcUrl, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'eth_chainId', params: [] }),
        signal: AbortSignal.timeout(1_000),
      });
      const payload = await response.json();
      if (payload.result === expectedChainId) return;
    } catch {}
    await delay(250);
  }
  throw new Error(`Anvil did not become ready at ${rpcUrl}`);
}

async function waitForHttp(url, label) {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1_000) });
      if (response.ok) return;
    } catch {}
    await delay(500);
  }
  throw new Error(`${label} did not become ready at ${url}`);
}

async function waitForFile(filePath, label) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (fs.existsSync(filePath)) return;
    await delay(250);
  }
  throw new Error(`${label} did not appear: ${filePath}`);
}

async function waitForLog(child, needle, label) {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`${label} exited early:\n${child.logs.join('')}`);
    if (child.logs.join('').includes(needle)) return;
    await delay(250);
  }
  throw new Error(`${label} did not report readiness:\n${child.logs.join('')}`);
}

async function requireCommand(command, args) {
  const child = await spawnLogged(command, args, { command, quiet: true });
  const code = await waitForExit(child);
  if (code !== 0) throw new Error(`${command} is required for local-chain QA but was not available`);
}

async function run(command, args, options = {}) {
  const child = await spawnLogged(options.name ?? command, args, { ...options, command });
  const code = await waitForExit(child);
  if (code !== 0) throw new Error(`${command} ${args.join(' ')} exited with code ${code}`);
}

async function spawnLogged(name, args, options = {}) {
  const child = spawn(options.command ?? name, args, {
    cwd: options.cwd ?? repoRoot,
    env: options.env ?? process.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  child.logs = [];
  const collect = (chunk) => {
    const text = chunk.toString();
    child.logs.push(text);
    if (child.logs.length > 80) child.logs.shift();
    if (!options.quiet) process.stdout.write(`[${name}] ${text}`);
  };
  child.stdout.on('data', collect);
  child.stderr.on('data', collect);
  child.once('error', (error) => { throw error; });
  return child;
}

async function waitForExit(child) {
  if (child.exitCode !== null) return child.exitCode;
  return new Promise((resolve) => child.once('exit', (code) => resolve(code ?? 1)));
}

async function stopProcesses() {
  for (const child of processes.reverse()) {
    if (child.exitCode !== null) continue;
    child.stdin?.end();
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      delay(2_000).then(() => child.kill('SIGTERM')),
      delay(4_000).then(() => child.kill('SIGKILL')),
    ]);
  }
}

function check(name, passed, detail) {
  return { name, passed: Boolean(passed), detail };
}

function txHash(value) {
  return typeof value === 'string' && /^0x[0-9a-f]{64}$/.test(value);
}

function signature(value) {
  return typeof value === 'string' && /^0x[0-9a-f]{130}$/.test(value);
}

function isIgnoredWalletbeatDevError(message) {
  return /generateFallbackAnalysis|Cannot read properties of undefined \(reading 'overall'\)|Failed to load resource: the server responded with a status of 404|Failed to load resource: the server responded with a status of 504 \(Outdated Optimize Dep\)|Failed to fetch dynamically imported module: .*node_modules\/\.vite\/deps\//.test(message);
}

function validateExtension() {
  for (const required of [path.join(extensionDir, 'manifest.json'), path.join(extensionDir, 'background.js'), path.join(extensionDir, 'injected.js')]) {
    if (!fs.existsSync(required)) throw new Error(`missing extension file: ${required}`);
  }
}

async function freePort() {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      server.close(() => resolve(address.port));
    });
  });
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
