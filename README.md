# Success CLI

A terminal-based goal tracking and productivity application built with Rust.

## Project Structure

- **`success-cli`** - CLI application (Ratatui-based TUI)
- **`success-core`** - Core business logic and data structures  
- **`success-web`** - Web version (Ratzilla/WebAssembly)repository)

## Getting Started

### Running the CLI Version

```bash
# Build and run the CLI application
cargo run --release

# Or run with a custom archive path
cargo run --release -- --archive /path/to/archive
```

The first time you run the CLI, it will prompt you to set an archive location where all your goals and sessions will be stored.

### Running the Web Version

The web version is built with Ratzilla (Rust + WebAssembly).

Web demo (GitHub Pages): https://calonca.github.io/success-cli/

**Recommended: Using Trunk** (all-in-one build tool for WASM)

```bash
# Install trunk
cargo install trunk

# Build and serve with hot reload (development)
cd success-web
trunk serve

# Build for production
trunk build --release
```

Then open `http://localhost:8080` in your web browser.

Then open the appropriate URL in your web browser (default is `http://localhost:8080` for trunk, `http://localhost:8000` for others).

### Running Ratzilla Web Examples

Ratzilla allows building terminal-themed web applications with Rust and WebAssembly.

```bash
# Change to the ratzilla directory
cd ratzilla

# Build and run an example (e.g., demo)
cargo run --example demo --target wasm32-unknown-unknown

# Other available examples
cargo run --example minimal
cargo run --example demo2
```

For more details on Ratzilla, see [ratzilla/README.md](ratzilla/README.md).

## Features

### CLI Application
- **Goal Management**: Create, track, and manage goals
- **Session Tracking**: Log work sessions and rewards
- **Progress Visualization**: View progress with visual progress bars
- **Notes**: Add and edit notes for each goal
- **External Editor**: Edit notes in your preferred text editor (press `E`)
- **Archive Management**: Open archive folder in file manager (press `o`)

### Key Bindings (CLI)
- `↑↓` - Navigate items
- `←→` - Change day
- `Enter` - Add session/confirm
- `e` - Edit notes (in-app)
- `E` - Edit notes (external editor)
- `o` - Open archive in file manager
- `Esc` - Cancel/exit

## Building

```bash
# Build debug version
cargo build

# Build release version
cargo build --release

# Run tests
cargo test

# Format code
cargo fmt

# Lint code
cargo clippy
```

## Configuration

The CLI stores its configuration at `~/.config/success-cli/config.json` which includes the path to your archive folder.

## Development

- Uses `Ratatui` for terminal UI
- Uses `Crossterm` for terminal handling
- Uses `Chrono` for date/time operations
- Modular architecture with core logic separated from UI
