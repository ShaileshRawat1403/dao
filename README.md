# DAO

**The AI-powered software engineering assistant.**

DAO is a deterministic, safety-first autonomous agent designed to help developers understand, plan, and execute software engineering tasks. Unlike "black box" AI agents, DAO follows a strict, observable state machine loop (Scan → Plan → Diff → Verify) ensuring you always remain in control.

## Features

- **🛡️ Safety First**: Built-in approval gates for execution and destructive actions. You decide what runs.
- **🔄 Deterministic Workflow**: Follows a structured lifecycle: `Idea` → `Understand` → `Plan` → `Preview` → `Approve` → `Verify`.
- **📼 Event Sourced**: Every action is recorded in an append-only log. Pause, resume, or replay any session to understand exactly what happened.
- **🎭 Personality Modes**:
  - **Friendly**: Verbose, impact-first explanations (great for onboarding).
  - **Pragmatic**: Terse, technical-first output (optimized for speed).
- **📦 Cross-Platform**: Native binaries for macOS (Intel/Apple Silicon), Linux, and Windows.

## Installation

### Automated Install (macOS/Linux)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/ShaileshRawat1403/dao/releases/latest/download/dao-installer.sh | sh
```

### Windows (PowerShell)

```powershell
irm https://github.com/ShaileshRawat1403/dao/releases/latest/download/dao-installer.ps1 | iex
```

### Manual Install

1. Download the archive for your platform from GitHub Releases.
2. Extract the archive.
3. Verify the checksum using the provided `.sha256` file.
4. Move the `dao` binary to your PATH.

```bash
dao --help
```

## Usage

### Running a Workflow

To start DAO on a repository, simply point it to the directory. It will scan the project and guide you through the workflow.

```bash
dao run --repo ./my-project
```

## Supported Platforms

- macOS (Intel & Apple Silicon)
- Linux (x64)
- Windows (x64)
