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
const profileDir = path.join(repoRoot, '.playwright', 'walletbeat-profile');
const bridgePort = Number(process.env.DECKARD_WALLETBEAT_BRIDGE_PORT ?? '8765');
const walletbeatPort = Number(process.env.DECKARD_WALLETBEAT_PORT ?? '8788');
const headed = process.env.DECKARD_WALLETBEAT_HEADED === '1';
const safeStepCount = Number(process.env.DECKARD_WALLETBEAT_SAFE_STEPS ?? '4');
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
    await page.locator('.tab-button').filter({ hasText: 'EIP Support' }).waitFor({
      state: 'visible',
      timeout: 15_000,
    });
    await page.waitForTimeout(1_500);
    await page.evaluate(() => {
      const tab = Array.from(document.querySelectorAll('.tab-button')).find((button) =>
        button.textContent?.includes('EIP Support'),
      );
      if (!(tab instanceof HTMLElement)) {
        throw new Error('WalletBeat EIP Support tab not found');
      }
      tab.click();
    });
    await page.waitForFunction(() => Boolean(document.querySelector('.step-test-container')), {
      timeout: 10_000,
    });

    const stepResults = [];
    for (let index = 0; index < safeStepCount; index += 1) {
      const stepResult = await runCurrentWalletbeatStep(page, index + 1);
      stepResults.push(stepResult);
    }

    const relevantPageErrors = pageErrors.filter((message) => !isIgnoredWalletbeatDevError(message));
    const ignoredPageErrors = pageErrors.filter(isIgnoredWalletbeatDevError);

    await page.screenshot({
      path: path.join(artifactsDir, 'walletbeat-safe-provider.png'),
      fullPage: true,
    });

    const report = {
      walletbeatRepo,
      walletbeatRef,
      url: `http://127.0.0.1:${walletbeatPort}/test/`,
      account: mockAccount,
      chainId: expectedChainId,
      safeStepCount,
      pageErrors: relevantPageErrors,
      ignoredPageErrors,
      stepResults,
    };
    fs.writeFileSync(
      path.join(artifactsDir, 'walletbeat-safe-provider-results.json'),
      `${JSON.stringify(report, null, 2)}\n`,
    );

    const criticalFailures = stepResults.flatMap((step) =>
      step.checks.filter((check) => check.critical && !check.passed).map((check) => ({
        step: step.title,
        check: check.name,
        detail: check.detail,
      })),
    );
    if (criticalFailures.length > 0) {
      throw new Error(`WalletBeat safe provider lane has critical failures:\n${JSON.stringify(criticalFailures, null, 2)}`);
    }
    if (relevantPageErrors.length > 0) {
      throw new Error(`WalletBeat page emitted console/page errors:\n${relevantPageErrors.join('\n')}`);
    }

    const summary = stepResults.map((step) => `${step.title}: ${step.status}`).join('\n');
    console.log(`WalletBeat safe provider lane passed critical checks:\n${summary}`);
  } finally {
    await context.close();
  }
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

async function runCurrentWalletbeatStep(page, stepNumber) {
  await page.locator('.sidebar-item').nth(stepNumber - 1).click();
  await page.locator('.step-card .actions button').first().click();
  await page.locator('.sidebar-item').nth(stepNumber - 1).locator('.sidebar-item-check').waitFor({
    state: 'visible',
    timeout: 15_000,
  });
  await page.waitForTimeout(300);

  const stepTitle = await page.locator('.sidebar-item').nth(stepNumber - 1).locator('h3').innerText();
  await page.locator('.sidebar-item').nth(stepNumber - 1).click();
  const statusIcon = await page.locator('.sidebar-item').nth(stepNumber - 1).locator('.sidebar-item-check').innerText();
  const status = statusIcon.includes('✓')
    ? 'passed'
    : statusIcon.includes('⚠')
      ? 'partial'
      : 'failed';
  const checks = await page.evaluate(() => {
    return Array.from(document.querySelectorAll('.check-item')).map((item) => {
      const statusText = item.querySelector('.check-status')?.textContent?.trim() ?? '';
      const nameElement = item.querySelector('.check-name');
      const name = Array.from(nameElement?.childNodes ?? [])
        .filter((node) => node.nodeType === Node.TEXT_NODE)
        .map((node) => node.textContent?.trim() ?? '')
        .join(' ')
        .trim() || nameElement?.textContent?.replace('Critical', '').trim() || '';

      return {
        name,
        status: statusText,
        passed: statusText.includes('✓') || statusText.includes('✅'),
        critical: Boolean(item.querySelector('.critical-badge')),
        detail: item.querySelector('.check-detail')?.textContent?.trim() ?? '',
      };
    });
  });

  return {
    title: stepTitle.trim(),
    status: status.trim(),
    checks,
  };
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
