import path from 'node:path';
import { spawnSync } from 'node:child_process';
function runGit(args, cwd) {
    const result = spawnSync('git', args, {
        cwd,
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'pipe'],
    });
    return {
        ok: result.status === 0,
        status: typeof result.status === 'number' ? result.status : -1,
        stdout: String(result.stdout || ''),
        stderr: String(result.stderr || ''),
    };
}
function normalizeGitRelativePath(gitRoot, absolutePath) {
    const relative = path.relative(gitRoot, absolutePath).replace(/\\/g, '/');
    if (!relative || relative.startsWith('..') || path.isAbsolute(relative)) {
        return '';
    }
    return relative;
}
export function resolveGitRoot(rootDir) {
    const probe = runGit(['-C', rootDir, 'rev-parse', '--show-toplevel'], rootDir);
    if (!probe.ok) {
        return '';
    }
    return path.resolve(String(probe.stdout || '').trim());
}
export function getGitDocState(rootDir, absoluteDocPath) {
    const gitRoot = resolveGitRoot(rootDir);
    const defaultState = {
        gitAvailable: Boolean(gitRoot),
        tracked: false,
        ignored: false,
        untracked: false,
        removedFromIndex: false,
        modified: false,
        staged: false,
        includeInRepoArchive: true,
        localOnlyArchive: false,
    };
    if (!gitRoot) {
        return defaultState;
    }
    const relativePath = normalizeGitRelativePath(gitRoot, absoluteDocPath);
    if (!relativePath) {
        return {
            ...defaultState,
            includeInRepoArchive: false,
        };
    }
    const trackedProbe = runGit(['-C', gitRoot, 'ls-files', '--error-unmatch', '--', relativePath], gitRoot);
    const tracked = trackedProbe.ok;
    const ignoredProbe = runGit(['-C', gitRoot, 'check-ignore', '-q', '--', relativePath], gitRoot);
    const ignored = ignoredProbe.status === 0;
    const statusProbe = runGit(['-C', gitRoot, 'status', '--porcelain=v1', '--', relativePath], gitRoot);
    const statusLines = statusProbe.ok
        ? String(statusProbe.stdout || '').split('\n').map((line) => line.trimEnd()).filter(Boolean)
        : [];
    let modified = false;
    let staged = false;
    let removedFromIndex = false;
    for (const line of statusLines) {
        const x = line[0] || ' ';
        const y = line[1] || ' ';
        // Keep untracked entries distinct from tracked modified files.
        if (x === '?' && y === '?') {
            continue;
        }
        if (x !== ' ' && x !== '?') {
            staged = true;
        }
        if (x === 'D') {
            removedFromIndex = true;
        }
        if (y !== ' ' || x === 'M' || x === 'A' || x === 'D' || x === 'R' || x === 'C') {
            modified = true;
        }
    }
    // Treat ignored, untracked, and index-removed docs as local-only backups.
    // This keeps accidental de-tracking from leaking into repository artifacts.
    const localOnlyArchive = ignored || !tracked || removedFromIndex;
    return {
        gitAvailable: true,
        tracked,
        ignored,
        untracked: !tracked && !ignored,
        removedFromIndex,
        modified,
        staged,
        includeInRepoArchive: !localOnlyArchive,
        localOnlyArchive,
    };
}
export function listGitEligibleDxFiles(rootDir) {
    const gitRoot = resolveGitRoot(rootDir);
    if (!gitRoot) {
        return null;
    }
    const trackedResult = runGit(['-C', gitRoot, 'ls-files', '-z', '--', '*.dx'], gitRoot);
    const untrackedResult = runGit(['-C', gitRoot, 'ls-files', '-z', '--others', '--exclude-standard', '--', '*.dx'], gitRoot);
    const eligible = new Set();
    const addFromOutput = (output) => {
        const entries = String(output || '').split('\0').filter(Boolean);
        for (const relativePath of entries) {
            const absolutePath = path.resolve(gitRoot, relativePath);
            const relativeToWorkspace = path.relative(rootDir, absolutePath);
            if (!relativeToWorkspace.startsWith('..') && !path.isAbsolute(relativeToWorkspace)) {
                eligible.add(absolutePath);
            }
        }
    };
    if (trackedResult.ok) {
        addFromOutput(trackedResult.stdout);
    }
    if (untrackedResult.ok) {
        addFromOutput(untrackedResult.stdout);
    }
    return eligible;
}
