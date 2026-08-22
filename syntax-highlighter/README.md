# Ontic VS Code Syntax Highlighting

TextMate grammar for `.ont` (wishes + programs) and `.sketch`
(candidate implementations).

## Install (local)

```bash
cd syntax-highlighter
npx @vscode/vsce package
code --install-extension ontic-language-0.1.0.vsix
```

Or symlink for development:

```bash
ln -s "$(pwd)/syntax-highlighter" ~/.vscode/extensions/ontic-language
```

## What gets colored

| Element | Example |
|---|---|
| declarations | `fn Ledger.total`, `program Demo` |
| tiers | `wrapping` |
| program blocks | `use`, `start`, `end` |
| evidence | `=>` transparent, `??` **opaque** |
| invariants | leading `\|` |
| variables | `%items`, `%res` |
| sketch symbols | `@total` (inside candidate text) |
| builtins | `len`, `fold`, `let`, `if/else`, `in`, `from` |
| types | `Int`, `Bool`, `List<Int>` |
