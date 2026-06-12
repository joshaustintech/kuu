# kuu

**Kuu** is a clean-room implementation of [Lua 5.5](https://www.lua.org/manual/5.5/) written in pure, safe Rust.

> [!NOTE]
> Kuu is an experimental project under construction. The goal is full Lua 5.5 conformance with zero external
> dependencies and zero unsafe code.

> [!NOTE]
> Built by various AI agents

## Constraints

- **No external dependencies** — `Cargo.toml` contains zero `[dependencies]`
- **No unsafe code** — enforced by `#![forbid(unsafe_code)]` in `src/main.rs`
- **Zero warnings** — all code passes `cargo clippy -- -D warnings`

## Usage

```sh
# Run a Lua script
kuu script.lua

# Evaluate a chunk directly
kuu -e 'print("hello, world")'
```

## Working Examples

### Hello, World

```lua
print("Hello, World!")
```

### Closures

```lua
local function counter(start)
  local n = start
  return function()
    n = n + 1
    return n
  end
end

local c = counter(0)
print(c())  --> 1
print(c())  --> 2
print(c())  --> 3
```

### Iterators

```lua
local function range(n)
  local i = 0
  return function()
    i = i + 1
    if i <= n then return i end
  end
end

for i in range(5) do
  io.write(i .. " ")
end
-- 1 2 3 4 5
```
