set shell := ["bash", "-eu", "-o", "pipefail", "-c"]

bin := "sksync"
install_dir := env_var_or_default("INSTALL_DIR", env_var("HOME") / ".local" / "bin")

# Show available recipes
_default:
    @just --list

# Build the release binary
build-release:
    cargo build --release

# Install sksync to INSTALL_DIR (default: ~/.local/bin)
install: build-release
    mkdir -p "{{install_dir}}"
    install -m 0755 "target/release/{{bin}}" "{{install_dir}}/{{bin}}"
    @echo "Installed {{bin}} to {{install_dir}}/{{bin}}"
    @echo "Ensure {{install_dir}} is on your PATH."

# Remove sksync from INSTALL_DIR (default: ~/.local/bin)
uninstall:
    rm -f "{{install_dir}}/{{bin}}"
    @echo "Removed {{install_dir}}/{{bin}}"
