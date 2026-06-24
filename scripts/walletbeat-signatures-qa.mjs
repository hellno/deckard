#!/usr/bin/env node
import { chromium } from '@playwright/test';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');

const walletbeatRepo = 'https://github.com/walletbeat/walletbeat.git';
const walletbeatRef =
  process.env.DECKARD_WALLETBEAT_REF ?? '4062044982c9e2878d64d8f38c59fdc87e276554';
const walletbeatDir = path.join(repoRoot, '.walletbeat', 'walletbeat');
const extensionDir = path.join(repoRoot, 'extension');
const artifactsDir = path.join(repoRoot, 'test-results', 'walletbeat');
const profileDir = path.join(repoRoot, '.playwright', 'walletbeat-signatures-profile');
const bridgePort = Number(process.env.DECKARD_WALLETBEAT_BRIDGE_PORT ?? '8765');
const walletbeatPort = Number(process.env.DECKARD_WALLETBEAT_PORT ?? '8788');
const headed = process.env.DECKARD_WALLETBEAT_HEADED === '1';
const mockAccount = '0xdec0ded000000000000000000000000000001193';
const chainIdDecimal = '11155111';
const expectedChainId = '0xaa36a7';
const processes = [];

main().catch(async (error) => {
  console.error(error instanceof Error ? error.stack ?? error.message : error);
  process.exitCode = 1;
}).finally(async () => {
  await stopProcesses();
});

async function main() {
  fs.mkdirSync(artifactsDir, { recursive: true });
  validateExtension();
  await ensureWalletbeatCheckout();
  await run('pnpm', ['install', '--frozen-lockfile'], { cwd: walletbeatDir });

  if (await bridgeReady()) {
    if (process.env.DECKARD_QA_REUSE_BRIDGE !== '1') {
      throw new Error(
        `Deckard browser bridge is already listening on 127.0.0.1:${bridgePort}; stop it or set DECKARD_QA_REUSE_BRIDGE=1`,
      );
    }
  } else {
    await startBridge();
  }

  const walletbeat = await spawnLogged(
    'pnpm',
    ['exec', 'astro', 'dev', '--host', '127.0.0.1', '--port', String(walletbeatPort)],
    {
      cwd: walletbeatDir,
      name: 'walletbeat',
      env: {
        ...process.env,
        ASTRO_TELEMETRY_DISABLED: '1',
        WALLETBEAT_DEV: 'true',
      },
    },
  );
  processes.push(walletbeat);
  await waitForHttp(`http://127.0.0.1:${walletbeatPort}/test/`, 'WalletBeat dev server');

  const context = await chromium.launchPersistentContext(profileDir, {
    channel: 'chromium',
    headless: !headed,
    serviceWorkers: 'allow',
    args: [
      `--disable-extensions-except=${extensionDir}`,
      `--load-extension=${extensionDir}`,
    ],
  });

  try {
    const page = await context.newPage();
    const pageErrors = [];
    page.on('pageerror', (error) => pageErrors.push(error.message));
    page.on('console', (message) => {
      if (message.type() === 'error') {
        pageErrors.push(message.text());
      }
    });

    await page.goto(`http://127.0.0.1:${walletbeatPort}/test/`, { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => Boolean(window.ethereum?.request), { timeout: 15_000 });

    const results = await page.evaluate(async ({ account, chainId }) => {
      const provider = window.ethereum;
      if (!provider) {
        throw new Error('window.ethereum missing');
      }
      const accounts = await provider.request({ method: 'eth_requestAccounts' });
      const activeChain = await provider.request({ method: 'eth_chainId' });
      const simple = await provider.request({
        method: 'personal_sign',
        params: [
          'This is a safe test message for educational purposes only. It does not authorize any transactions or actions.',
          account,
        ],
      });
      const siwe = await provider.request({
        method: 'personal_sign',
        params: [
          `${window.location.host} wants you to sign in with your Ethereum account:\n${account}\n\nSign-In With Ethereum test for Deckard WalletBeat QA.\n\nURI: ${window.location.origin}\nVersion: 1\nChain ID: ${chainId}\nNonce: deckardqa1\nIssued At: 2026-01-01T00:00:00.000Z`,
          account,
        ],
      });
      const typed = await provider.request({
        method: 'eth_signTypedData_v4',
        params: [
          account,
          {
            domain: {
              name: 'Test Signature App',
              version: '1',
              chainId,
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
              purpose: 'Educational Testing Only',
              message: 'This signature is for testing purposes only. It does not authorize any transactions, transfers, or approvals.',
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
      return { accounts, activeChain, simple, siwe, typed, ethSignError };
    }, { account: mockAccount, chainId: Number(chainIdDecimal) });

    const checks = [
      signatureCheck('simple personal_sign', results.simple),
      signatureCheck('SIWE personal_sign', results.siwe),
      signatureCheck('EIP-712 typed data', results.typed),
      {
        name: 'eth_sign refused',
        passed: results.ethSignError?.code === 4200 && /raw eth_sign/.test(results.ethSignError?.message ?? ''),
        detail: results.ethSignError,
      },
      {
        name: 'connected account',
        passed: Array.isArray(results.accounts) && results.accounts[0] === mockAccount,
        detail: results.accounts,
      },
      {
        name: 'active chain',
        passed: results.activeChain === expectedChainId,
        detail: results.activeChain,
      },
    ];

    await page.screenshot({
      path: path.join(artifactsDir, 'walletbeat-signatures.png'),
      fullPage: true,
    });

    const relevantPageErrors = pageErrors.filter((message) => !isIgnoredWalletbeatDevError(message));
    const ignoredPageErrors = pageErrors.filter(isIgnoredWalletbeatDevError);
    const report = {
      walletbeatRepo,
      walletbeatRef,
      url: `http://127.0.0.1:${walletbeatPort}/test/`,
      account: mockAccount,
      chainId: expectedChainId,
      checks,
      ignoredPageErrors,
      pageErrors: relevantPageErrors,
    };
    fs.writeFileSync(
      path.join(artifactsDir, 'walletbeat-signatures-results.json'),
      `${JSON.stringify(report, null, 2)}\n`,
    );

    const failures = checks.filter((check) => !check.passed);
    if (failures.length > 0) {
      throw new Error(`WalletBeat signatures lane failed:\n${JSON.stringify(failures, null, 2)}`);
    }
    if (relevantPageErrors.length > 0) {
      throw new Error(`WalletBeat page emitted console/page errors:\n${relevantPageErrors.join('\n')}`);
    }

    console.log(`WalletBeat signatures lane passed:\n${checks.map((check) => `${check.name}: passed`).join('\n')}`);
  } finally {
    await context.close();
  }
}

function signatureCheck(name, signature) {
  return {
    name,
    passed: typeof signature === 'string' && /^0x[0-9a-f]{130}$/.test(signature),
    detail: typeof signature === 'string' ? `${signature.slice(0, 10)}…${signature.slice(-8)}` : signature,
  };
}

async function ensureWalletbeatCheckout() {
  fs.mkdirSync(path.dirname(walletbeatDir), { recursive: true });
  if (!fs.existsSync(path.join(walletbeatDir, '.git'))) {
    await run('git', ['clone', '--filter=blob:none', walletbeatRepo, walletbeatDir], { cwd: repoRoot });
  }
  await run('git', ['fetch', '--depth=1', 'origin', walletbeatRef], { cwd: walletbeatDir });
  await run('git', ['checkout', '--detach', 'FETCH_HEAD'], { cwd: walletbeatDir });
}

async function startBridge() {
  const bridge = await spawnLogged(
    'cargo',
    [
      'run',
      '--quiet',
      '--locked',
      '-p',
      'deckard-browser-bridge',
      '--no-default-features',
      '--',
      '--bind',
      `127.0.0.1:${bridgePort}`,
      '--dev-mock-account',
      mockAccount,
    ],
    {
      cwd: repoRoot,
      name: 'bridge',
      env: {
        ...process.env,
        DECKARD_CHAIN_ID: chainIdDecimal,
      },
    },
  );
  processes.push(bridge);
  await waitForBridge();
}

async function waitForBridge() {
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    if (await bridgeReady()) {
      return;
    }
    await delay(250);
  }
  throw new Error(`Deckard browser bridge did not become ready on 127.0.0.1:${bridgePort}`);
}

async function bridgeReady() {
  try {
    const response = await fetch(`http://127.0.0.1:${bridgePort}/rpc`, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'x-deckard-origin': `http://127.0.0.1:${walletbeatPort}`,
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'eth_chainId',
        params: [],
      }),
      signal: AbortSignal.timeout(1_000),
    });
    if (!response.ok) {
      return false;
    }
    const payload = await response.json();
    return payload.result === expectedChainId;
  } catch {
    return false;
  }
}

async function waitForHttp(url, label) {
  const deadline = Date.now() + 60_000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(url, { signal: AbortSignal.timeout(1_000) });
      if (response.ok) {
        return;
      }
    } catch {
      // Keep polling until the dev server is ready.
    }
    await delay(500);
  }
  throw new Error(`${label} did not become ready at ${url}`);
}

async function run(command, args, options = {}) {
  const child = await spawnLogged(command, args, options);
  const code = await waitForExit(child);
  if (code !== 0) {
    throw new Error(`${command} ${args.join(' ')} exited with code ${code}`);
  }
}

async function spawnLogged(command, args, options = {}) {
  const child = spawn(command, args, {
    cwd: options.cwd ?? repoRoot,
    env: options.env ?? process.env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const name = options.name ?? command;
  child.stdout.on('data', (chunk) => process.stdout.write(`[${name}] ${chunk}`));
  child.stderr.on('data', (chunk) => process.stderr.write(`[${name}] ${chunk}`));
  child.once('error', (error) => {
    throw error;
  });
  return child;
}

async function waitForExit(child) {
  if (child.exitCode !== null) {
    return child.exitCode;
  }
  return new Promise((resolve) => {
    child.once('exit', (code) => resolve(code ?? 1));
  });
}

async function stopProcesses() {
  await Promise.all(processes.reverse().map(async (child) => {
    if (child.exitCode !== null) {
      return;
    }
    child.kill('SIGTERM');
    await Promise.race([
      new Promise((resolve) => child.once('exit', resolve)),
      delay(2_000).then(() => child.kill('SIGKILL')),
    ]);
  }));
}

function validateExtension() {
  for (const required of [
    path.join(extensionDir, 'manifest.json'),
    path.join(extensionDir, 'background.js'),
    path.join(extensionDir, 'injected.js'),
  ]) {
    if (!fs.existsSync(required)) {
      throw new Error(`missing extension file: ${required}`);
    }
  }
}

function isIgnoredWalletbeatDevError(message) {
  return (
    message.includes('Outdated Optimize Dep') ||
    (
      message.includes('Failed to fetch dynamically imported module:') &&
      message.includes('/node_modules/.vite/deps/')
    )
  );
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
