# Wasm Examples for Hyperlight

Example WebAssembly modules demonstrating Hyperlight sandbox usage.

## Examples

### 1. Hello World (`hello.wat`)
Simple WASI example that prints "Hello from Wasm!"

### 2. Calculator (`calc.wat`)
Basic arithmetic operations: add, sub, mul, div, mod, factorial

### 3. Fibonacci (`fib.wat`)
Recursive and iterative Fibonacci implementations

## Running with Hyperlight

agentkernel natively supports both `.wasm` (binary) and `.wat` (text) formats.
WAT files are automatically compiled to WASM on load.

```bash
# Run .wat file directly (auto-compiled)
agentkernel run -B hyperlight -i examples/wasm/hello.wat -- _start

# Run pre-compiled .wasm file
agentkernel run -B hyperlight -i examples/wasm/calc.wasm -- add 5 3

# Use the daemon for faster repeated execution
agentkernel daemon start
agentkernel run -i examples/wasm/fib.wat -- fib_iterative 10
```

## Manual Compilation (Optional)

If you need to compile WAT files manually:

```bash
# Install wabt (WebAssembly Binary Toolkit)
brew install wabt  # macOS
apt-get install wabt  # Ubuntu

# Convert .wat to .wasm
wat2wasm hello.wat -o hello.wasm
```

## Notes

- Hyperlight provides sub-millisecond startup times for Wasm modules
- Each module runs in complete isolation (KVM-based)
- WAT files are compiled once and cached
- No filesystem or network access by default (secure by design)
- Requires Linux with KVM support
