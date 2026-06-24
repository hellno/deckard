#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..');
const walletbeatRoot = path.join(repoRoot, '.walletbeat', 'walletbeat');
const matrixPath = path.join(repoRoot, 'docs', 'WALLETBEAT-SAFETY-MATRIX.md');
const scamPath = path.join(walletbeatRoot, 'src', 'constants', 'test-scam-alerts.ts');
const simulationPath = path.join(walletbeatRoot, 'src', 'components', 'Tabs', 'TransactionSimulationsTab.svelte');

const requiredIssues = new Set(['#73', '#74', '#135', '#149', '#150', '#151', '#152', '#153']);

function read(file) {
  return fs.readFileSync(file, 'utf8');
}

function ensureWalletbeatCheckout() {
  for (const file of [scamPath, simulationPath]) {
    if (!fs.existsSync(file)) {
      throw new Error(`WalletBeat checkout missing ${path.relative(repoRoot, file)}. Run pnpm run qa:walletbeat first to populate .walletbeat/.`);
    }
  }
}

function scamFixtureIds(source) {
  return [...source.matchAll(/id:\s*'([^']+)'/g)].map((match) => match[1]);
}

function simulationFixtureIds(source) {
  const typeMatch = source.match(/export type TransactionSimulationSubTab =([\s\S]*?);/);
  if (!typeMatch) throw new Error('Could not find TransactionSimulationSubTab union');
  return [...typeMatch[1].matchAll(/'([^']+)'/g)].map((match) => match[1]);
}

function matrixRows(markdown) {
  const rows = new Map();
  for (const line of markdown.split('\n')) {
    if (!line.startsWith('| `')) continue;
    const cells = line.split('|').slice(1, -1).map((cell) => cell.trim());
    if (cells.length < 5) continue;
    const id = cells[0].replace(/^`|`$/g, '');
    rows.set(id, { status: cells[2], followup: cells[4], line });
  }
  return rows;
}

function issuesIn(text) {
  return new Set([...text.matchAll(/#\d+/g)].map((match) => match[0]));
}

function main() {
  ensureWalletbeatCheckout();
  const matrix = read(matrixPath);
  const rows = matrixRows(matrix);
  const expected = [
    ...scamFixtureIds(read(scamPath)),
    ...simulationFixtureIds(read(simulationPath)),
  ];
  const failures = [];

  for (const id of expected) {
    if (!rows.has(id)) failures.push(`missing matrix row for WalletBeat fixture ${id}`);
  }
  for (const id of rows.keys()) {
    if (!expected.includes(id)) failures.push(`matrix row ${id} is not present in pinned WalletBeat fixtures`);
  }
  for (const [id, row] of rows) {
    if (/untracked gap/i.test(row.status) || /untracked gap/i.test(row.followup)) {
      failures.push(`matrix row ${id} still contains an untracked gap marker`);
    }
  }

  for (const [id, row] of rows) {
    if (/tracked gap|blocked/i.test(row.status) && !/#\d+/.test(row.followup)) {
      failures.push(`matrix row ${id} is ${row.status} but has no linked follow-up issue`);
    }
  }

  const matrixIssues = issuesIn(matrix);
  for (const issue of requiredIssues) {
    if (!matrixIssues.has(issue)) failures.push(`matrix is missing required issue link ${issue}`);
  }

  if (failures.length > 0) {
    console.error('WalletBeat safety matrix QA failed:');
    for (const failure of failures) console.error(`- ${failure}`);
    process.exit(1);
  }

  const tracked = [...rows.values()].filter((row) => /tracked gap/i.test(row.status)).length;
  const safe = [...rows.values()].filter((row) => /safe refusal/i.test(row.status)).length;
  const supported = [...rows.values()].filter((row) => /supported/i.test(row.status)).length;
  console.log('WalletBeat safety matrix QA passed:');
  console.log(`fixtures covered: ${expected.length}`);
  console.log(`supported: ${supported}`);
  console.log(`safe refusal: ${safe}`);
  console.log(`tracked gaps: ${tracked}`);
}

main();
