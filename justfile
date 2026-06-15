# cosmic-applet-system-monitor — build & install recipes
# Usage: `just build`, `just install`, `just enable`

rootdir := ''
prefix := '/usr'
bin := 'cosmic-applet-system-monitor'
appid := 'com.system76.CosmicAppletSystemMonitor'

bin-dst := rootdir + prefix + '/bin/' + bin
desktop-dst := rootdir + prefix + '/share/applications/' + appid + '.desktop'

# Build the optimized release binary
build:
    cargo build --release

# Run the test suite
test:
    cargo test --all-features

# Format check + clippy + tests (mirrors CI)
check:
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features -- -D warnings
    cargo test --all-features

# Install the binary and desktop entry (uses sudo)
install: build
    sudo install -Dm0755 target/release/{{bin}} {{bin-dst}}
    sudo install -Dm0644 resources/{{appid}}.desktop {{desktop-dst}}

# Remove installed files
uninstall:
    sudo rm -f {{bin-dst}} {{desktop-dst}}

# Add the applet to the running COSMIC panel and restart it
enable:
    #!/usr/bin/env bash
    set -euo pipefail
    cfg="$HOME/.config/cosmic/com.system76.CosmicPanel.Panel/v1/plugins_wings"
    echo "Add \"{{appid}}\" to the panel via Settings → Desktop → Panel → Applets,"
    echo "or edit: $cfg"

clean:
    cargo clean
