Depot
=====
A tragic solution to a terrible situation
-----------------------------------------

Cargo is Rust's default package manager.

Cargo also wants to be Rust's default compiler frontend.

Cargo always assumes it pulls in dependencies for a single project.

By the reasoning of the adeptus mechanicus, one should use Cargo for all projects, irregardless of if they need to use the same binaries.

This is where things go bad. Rust's already geologic compiletimes are exacerbated by recompiling **THE SAME GODDAMN BINARY FOR THE SAME GODDAMN LIBRARY EVERY TIME IT'S USED.**

![](http://i.imgur.com/6wVMkUl.jpg)

**Depot** solves this by aggregating the list of the libraries you need to share between projects and telling Cargo to build a dummy lib that 'requires' them. It either accepts commandline arguments, or parses a Depot.toml, which looks like this:

```toml
  [depot]
  name = "depot"            # Name of the depot project.
  dirs = [                  # Directories that contain a Dependencies.toml
    "./somewhere_with_dependencies"
  ]
  out-dir = "./somewhere_else"

  # Unlike Cargo, if you leave this section out, libraries will be built optimized up the yingyang.
  [settings]
  opt-level = 0             # Controls the --opt-level the compiler builds with
  debug = true              # Controls whether the compiler passes -g or `--cfg ndebug`
  debug-assertions = true   # Controls whether debug assertions are enabled
```

It searches in the dirs specified for Dependencies.tomls, which look like Cargo's [dependencies] section:

```toml
  [dependencies.sdl2]
  git = "https://github.com/AngryLawyer/rust-sdl2.git"
```

These

Oh my glob, this code is horrendous.
------------------------------------

It was written in a blazing stream of fury. Since this is a heavily requested feature in Cargo, I'm a little less enthused about taking the time to structure this properly until a decision is reached there.

So for now, it's a hack!

How do I clean the depot?
-------------------------
Run cargo clean in the hidden yourdepotname/.yourdepotname-cargoproject directory.

Yes.

stop looking at me that way