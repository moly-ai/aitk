# Important context
- AITK is a Rust crate providing cross-platform and framework-agnostic abstractions to work with AI/LLMs.
- It provides traits, core types, API clients, state management utilities, and more.
- Works on desktop, mobile, and web (WASM). It can be used in CLI apps, GUI apps, servers, etc.

# Code style
- Avoid unnecessary or obvious comments.
- Favor simple and elegant solutions over over-engineered ones.
- Keep library code generic and reusable.
- Limit line length to 100 characters (rustfmt default).
- Maximize code reuse (DRY).
- No extra code beyond what is absolutely necessary to solve the problem the user provides. Ask for missing details
  and suggest next steps to the user if appropriate.

# Documentation

- Must include doc comments for all public functions, structs, enums, and methods in library code.
- Must document errors and panics where applicable.
- Keep comments up-to-date with code changes.

# Type System

- Must leverage Rust's type system to prevent bugs at compile time.
- Use newtypes to distinguish semantically different values of the same underlying type.
- Prefer `Option<T>` over sentinel values.

# Error Handling

- Never use .unwrap() in library code; use .expect() only for invariant violations with a descriptive message.
- Define meaningful error types in library code.

# Function Design

- Must keep functions focused on a single responsibility.
- If ownership is not required, prefer borrowing parameters (&T, &mut T).
- If ownership is required, prefer taking parameters by value (T) over references (which would end up in unnecessary cloning).
- Limit function parameters to 5 or fewer; use a config struct for more.
- Return early to reduce nesting.

# Struct and Enum Design

- Must keep types focused on a single responsibility.
- Must derive common traits: Debug, Clone, PartialEq where appropriate.
- Use `#[derive(Default)]` when a sensible default exists.
- Prefer composition over inheritance-like patterns.
- Use builder pattern for complex struct construction.

# Rust Best Practices

- Never use unsafe unless absolutely necessary; document safety invariants when used.
- Must use pattern matching exhaustively; avoid catch-all _ patterns when possible.
- Must use format! macro for string formatting.
- Use iterators and iterator adapters over manual loops.
- Use enumerate() instead of manual counter variables.
- Prefer if let and while let for single-pattern matching.

# Memory and Performance

- Must avoid unnecessary allocations; prefer &str over String when possible.
- Must use Cow<'_, str> when ownership is conditionally needed.
- Use Vec::with_capacity() when the size is known.
- Prefer stack allocation over heap when appropriate.
- Use Arc and Rc judiciously; prefer borrowing.

# Async

- Must return futures for the user to spawn with their runtime of choice, instead of spawning tasks internally in library code.
  Unless, there is a buisness need to do otherwise (i.e. the controller abstraction for state management).
- Must use primitives of the `futures` crate over `tokio` ones (i.e. use futures' channels over tokio's channels).
- Do not use tokio for cross-platform code that needs to run on web. Prefer abstractions that work on native and web (WASM) seamlessly, like the ones provided by the utilities of this crate.
