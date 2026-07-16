# td

A minimalist terminal todo list. Vim-style keys, no clutter, no accounts, no sync. Press `?` for help!

```
 td                                                        1/3 · due · dark

  [ ] Buy milk                                                      tomorrow
      Two percent, from the corner shop. Also grab oat milk if they
      have it in stock today.
› [ ] Ship the report                                               in 3d
  [x] Book the dentist

 j/k move · space done · o new · t due · a file · h history · ? help
```

## Install

You'll need a Rust toolchain ([rustup.rs](https://rustup.rs)). Then:

```sh
cargo install --git https://github.com/leungchristopher/todo
```

Or from a clone:

```sh
git clone https://github.com/leungchristopher/todo
cd todo
cargo install --path .
```

Either way you get a `td` command in `~/.cargo/bin`. Type `td` to start.

## Features

- Simple todos with a checkbox, title, optional due date, and details
- Due dates in natural language.
- History, swept by hand: keeps you motivated (type `:morning` to sweep all done tasks into history!). Press `a` to file a todo away by hand, ticked or not. Press `h` to read history, and `a` in there to pull something back onto the list.

Todos live in `~/.local/share/td/todos.txt`, one per line. Grep it, diff it, put it in a repo. Set `TD_FILE` to keep it somewhere else.

## Configs

`~/.config/td/config`, written for you on first change:

```
theme=dark        # dark | light
sort=created      # created | due | alpha
phrase=morning    # what you type after : to sweep ticked todos to history
```

`m` and `s` toggle the first two while you work. The phrase is yours to edit. It takes anything, spaces included, so `phrase=rise and shine` is fair game!

## License

MIT
