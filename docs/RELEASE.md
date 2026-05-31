# Release Runbook

Use this runbook when preparing and publishing a sksync release.

## Normal release

1. Pick the next version, usually the next patch version.
2. Update package and documentation version references:
   - `Cargo.toml`
   - `Cargo.lock`
   - manual site version in `site/.vitepress/config.ts`
   - generated lockfile examples in `site/guides/lockfile.md` and `docs/DESIGN.md`
3. Run verification before opening the release PR:

   ```bash
   cargo fmt --check
   cargo test --quiet
   cargo build --release --quiet
   cargo clippy --quiet -- -D warnings
   bun install
   bun run docs:build
   ```

4. Open and merge a release PR.
5. Tag the merge commit and push the tag:

   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

6. Watch the `release` workflow and confirm all expected assets appear on the GitHub Release:
   - `sksync-aarch64-apple-darwin.tar.gz`
   - `sksync-x86_64-apple-darwin.tar.gz`
   - `sksync-aarch64-unknown-linux-musl.tar.gz`
   - `sksync-x86_64-unknown-linux-musl.tar.gz`
   - `checksums.txt`

## macOS runner fallback

GitHub-hosted macOS runners can remain queued. If the Linux jobs have completed and the macOS jobs are still queued long enough that waiting is not useful, publish the macOS assets locally instead of leaving the release incomplete.

1. Cancel the stuck workflow run:

   ```bash
   gh run cancel <run-id>
   ```

2. From a clean checkout at the release tag, install the macOS targets:

   ```bash
   rustup target add aarch64-apple-darwin x86_64-apple-darwin
   ```

3. Build and package both macOS assets:

   ```bash
   rm -rf dist-local-macos
   mkdir -p dist-local-macos

   for target in aarch64-apple-darwin x86_64-apple-darwin; do
     cargo build --release --locked --target "$target" --quiet
     staging="dist-local-macos/sksync-$target"
     mkdir -p "$staging"
     cp "target/$target/release/sksync" "$staging/sksync"
     tar -C "$staging" -czf "dist-local-macos/sksync-$target.tar.gz" sksync
     "target/$target/release/sksync" --version
   done
   ```

4. Generate macOS checksums:

   ```bash
   cd dist-local-macos
   shasum -a 256 *.tar.gz > macos-checksums.txt
   cd ..
   ```

5. Merge existing Linux checksums with the local macOS checksums:

   ```bash
   rm -rf /tmp/sksync-release-checksums
   mkdir /tmp/sksync-release-checksums
   gh release download vX.Y.Z --pattern checksums.txt --dir /tmp/sksync-release-checksums
   cat /tmp/sksync-release-checksums/checksums.txt \
     dist-local-macos/macos-checksums.txt \
     | sort -k2 > dist-local-macos/checksums.txt
   ```

6. Upload the macOS assets and refreshed checksums:

   ```bash
   gh release upload vX.Y.Z \
     dist-local-macos/sksync-aarch64-apple-darwin.tar.gz \
     dist-local-macos/sksync-x86_64-apple-darwin.tar.gz \
     dist-local-macos/checksums.txt \
     --clobber
   ```

7. Confirm the release contains all four platform archives and the combined `checksums.txt`.

## GitButler workspace sync

After the release PR is merged and any tag/release work is complete, sync the main GitButler workspace with:

```bash
but pull --check
but pull --status-after
```
