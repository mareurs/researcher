# Language Patterns

Per-language anti-patterns and correct patterns for this project's languages.
Each section lists the top 5 mistakes LLMs make and the top 5 idiomatic patterns.

### Python

**Anti-patterns (Don't → Do):**
1. Mutable default arguments `def f(items=[])` → use `None` with `if items is None: items = []`
2. `typing.List`, `typing.Dict`, `typing.Optional` → built-in generics: `list[str]`, `str | None`
3. Bare/broad exception handling `except Exception: pass` → catch specific exceptions, log with context
4. `os.path.join()` → `pathlib.Path`: `Path(base) / "data" / "file.csv"`
5. `Any` type overuse → complete type annotations on all function signatures

**Correct patterns:**
1. Modern type hints (3.10+): `list[int]`, `dict[str, Any]`, `str | None`
2. `uv` for packages, `ruff` for linting/formatting, `pyright` for types, `pytest` for testing
3. `pyproject.toml` over `setup.py`/`requirements.txt`
4. `dataclasses` for internal data, Pydantic for validation, TypedDict for dict shapes
5. `is` comparison for singletons: `if x is None:` not `if x == None:`

---

### Rust

**Anti-patterns (Don't → Do):**
1. Gratuitous `.clone()` to silence borrow checker → borrow: `&str` over `&String`, `&[T]` over `&Vec<T>`
2. `.unwrap()` everywhere → `?` with `.context()` from anyhow, `.expect("invariant: ...")` only for proven invariants
3. `Rc<RefCell<T>>` / interior mutability overuse → restructure data flow and ownership
4. `String` params where `&str` suffices → `fn greet(name: &str)`, use `Cow<'_, str>` when ownership is conditional
5. Catch-all `_ => {}` in match → handle all variants explicitly, let compiler check exhaustiveness

**Correct patterns:**
1. `thiserror` for library errors, `anyhow` for application errors — propagate with `?`
2. Iterator chains over explicit loops — `.iter().map(f).collect()`, avoid unnecessary `.collect()`
3. `Vec::with_capacity()` when size is known
4. Derive common traits: `#[derive(Debug, Clone, PartialEq)]`, `#[derive(Default)]` when sensible
5. `if let`/`while let` for single-pattern matching instead of full match
