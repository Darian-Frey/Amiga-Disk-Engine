# Improvements

Catalogue of code-quality improvements, refactors, and architectural changes proposed during development. Per Maintenance Rule 8, improvements are logged here when noticed, not silently applied. The author decides whether to apply, defer, or decline.

This is the dual of [BUGS.md](BUGS.md): bugs are broken; improvements work but could be better.

Status vocabulary: suggested | applied | declined | deferred.
Effort vocabulary: trivial | small | medium | large.

> First entry logged 2026-08-22. Use `IMP-001`, `IMP-002`, … sequentially. Every entry needs a **Trade-offs** field, or it is a feature request, not an improvement candidate.

## Suggested

_None._

## Applied

### IMP-001 `ade ls` output is not machine-parseable
**Status:** applied
**Effort:** small
**Found:** 2026-08-22, while validating extraction across the corpus.
**Where:** [cli/src/main.rs](../cli/src/main.rs), `list`.

**What works today.** The listing is readable and correct: size, protection bits, datestamp, name, optional comment, aligned in columns.

**Why it could be better.** The columns are whitespace-separated and two fields contain spaces — the datestamp (`1990-09-20 17:10:20`) and, more awkwardly, Amiga filenames, which routinely contain spaces. Splitting on whitespace therefore cannot recover the name. I hit this writing a validation script and misread 4283 files as extraction failures before realising the parser, not the tool, was wrong.

F-015 commits to "a documented, versioned CLI and library binding suitable for automation, with stable exit codes and **structured output**". The exit codes are done; the structured output is not.

**Suggested fix.** A `--format` flag offering at least a machine-readable mode. Tab-separated with the name last is the cheap option; JSON Lines is the more useful one for F-014's batch reporting, and would suit `ade info` equally, whose evidence list and fault list are currently prose.

**Trade-offs.** A second output path is a second thing to keep correct, and committing to a schema early risks having to break it — which for a stability promise is worse than not having made it. Against that: the schema is easier to design now, while the only consumers are ADE's own tests, than after F-014 has scripts depending on the human format. Doing nothing means the scriptable surface F-015 promises does not exist, and anyone automating ADE will parse the human output and be broken by the first column change.

**Related.** F-014 (batch reporting needs machine-readable summaries), F-015 (the promise itself).

**Applied 2026-08-22.** A `--format=json` flag on `ade info` and `ade ls`, with `--format=text` remaining the default and explicitly documented as unstable.

`ls` emits JSON Lines — one object per entry, so a large directory streams rather than buffering. `info` emits a single object carrying its evidence and fault lists as real arrays instead of prose. Names are written as `\uXXXX` escapes, so output is **pure ASCII** and a Latin-1 filename round-trips losslessly: a name is a sequence of bytes on the disk, not text, and a consumer must be able to get the original bytes back.

Written with an internal `ade_core::json` writer rather than `serde`, keeping the workspace at zero dependencies. The scope is narrow — ADE emits JSON and never parses it — so the whole escaping surface is one function.

**Two things came out of doing it.**

Fault computation moved from the CLI into `Inspection::faults()`, with a typed `Fault` carrying a **stable code** alongside its message. The message is for people and may be reworded; the code is the contract. That also means the human and JSON outputs cannot drift apart about what is wrong with an image, and a future GUI inherits the same list.

It exposed a genuine bug: `ade ls --format=json disk.adf | head` **panicked**, because `println!` panics on a closed pipe. For a tool designed to be piped that is unacceptable. Restoring the default `SIGPIPE` disposition needs a `libc` call behind `unsafe`, which the workspace forbids, so all output now goes through an `emit` helper that treats a closed pipe as the ordinary end of a command.

**Verified** across all 4288 corpus images: 4288 `info` objects and 68,961 JSON Lines, every one parsing, zero non-ASCII bytes, zero malformed output.


## Declined

_None._

## Deferred

_None._
