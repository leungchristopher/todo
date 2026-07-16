# td

A minimalist terminal todo list. Vim-style keys, no clutter, no accounts, no sync. Press `?` for help!

```
 td  1 todos  2 projects                                   1/3 · due · dark

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
- Projects, on the second tab. `1` and `2` switch between the two.

Todos live in `~/.local/share/td/todos.txt`, one per line. Grep it, diff it, put it in a repo. Set `TD_FILE` to keep it somewhere else.

## Projects

Press `2`. A project consists of a name and a log: somewhere to write down where you got to.

```
 td  1 todos  2 projects                                    3 logged · dark

› td rewrite                                                        today
  conference talk                                                      3d
  flat move                                                         empty

 j/k move · enter open log · o new · e rename · 1 todos · ? help
```

`o` starts one, `e` renames, `x` deletes (`u` undoes). The right column is when you last wrote in it: avoid stale projects!

`enter` opens the log in a custom vim environment.

```
 i I a A o O   insert
 esc           back to normal mode
 h j k l       move · also arrows, 0 $ w b gg G
 x D dd        cut character · to end of line · line
 u             undo
 :w  :wq  ZZ   save and close
 :q            close · :q! to throw the edit away
```

Projects live in `~/.local/share/td/projects.txt`, next to the todos, one project per line with newlines escaped as `\n`.

## Configs

`~/.config/td/config` gives you:

```
theme=dark        # dark | light
sort=created      # created | due | alpha
phrase=morning    # what you type after : to sweep ticked todos to history
```

`m` and `s` toggle the first two while you work. The phrase is yours to edit. It takes anything, spaces included, so `phrase=rise and shine` is fair game!

## License

MIT
