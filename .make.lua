-- nastty's build, as recipes for oslo's `make` builtin.
--
--   oslo make            the recipes, with what each of them says it does
--   oslo make build      the release binary
--   oslo make verify     the whole local gate

local make = oslo.make

---------------------------------------------------------------------------- what the build is

local manifest = assert(oslo.fs.read("Cargo.toml"), "no Cargo.toml; is this a nastty checkout?")
local package = manifest:match("%[package%](.-)\n%[") or manifest
local NAME = assert(package:match('\nname%s*=%s*"([^"]+)"'), "no name in Cargo.toml")
local VERSION = assert(package:match('\nversion%s*=%s*"([^"]+)"'), "no version in Cargo.toml")
local PREFIX = os.getenv("PREFIX") or (os.getenv("HOME") .. "/.local")
local BIN = "target/release/" .. NAME
local SOURCES = { "src/**/*.rs", "examples/**/*.rs", "Cargo.toml", "Cargo.lock" }

local function cargo(...)
  return sh.cargo(...)
end

-- Words from a free-form `--args` parameter, for `run`.
local function words(text)
  local out = {}
  for word in (text or ""):gmatch("%S+") do out[#out + 1] = word end
  return out
end

---------------------------------------------------------------------------- building

make.recipe{ name = "version", desc = "what this checkout calls itself",
             run = function() print(("%s v%s"):format(NAME, VERSION)) end }

make.recipe{
  name = "build",
  desc = "the optimised release binary",
  inputs = SOURCES,
  outputs = { BIN },
  stale = "content",
  run = function() cargo("build", "--release", "--bin", NAME) end,
}
make.alias("b", "build")

make.recipe{ name = "dev", desc = "a debug build of the nastty executable",
             run = function() cargo("build", "--bin", NAME) end }

make.recipe{
  name = "run",
  desc = "run a development example (--example main)",
  params = { { "--example", desc = "which example", default = "main" } },
  run = function(a) cargo("run", "--example", a.example) end,
}
make.alias("r", "run")

make.recipe{
  name = "serve",
  desc = "run 'nastty serve' (--args for its flags)",
  params = { { "--args", desc = "arguments for nastty serve", default = "" } },
  run = function(a) cargo("run", "--bin", NAME, "--", "serve", table.unpack(words(a.args))) end,
}

make.recipe{
  name = "tui",
  desc = "run 'nastty tui' (--args for its flags)",
  params = { { "--args", desc = "arguments for nastty tui", default = "" } },
  run = function(a) cargo("run", "--bin", NAME, "--", "tui", table.unpack(words(a.args))) end,
}

make.recipe{ name = "tui-preview", desc = "render development snapshots for every TUI workspace",
             run = function() cargo("test", "--lib", "preview", "--", "--ignored", "--nocapture") end }

make.recipe{ name = "clean", desc = "remove the Cargo artifacts",
             run = function() cargo("clean") end }

make.recipe{ name = "compile", desc = "clean, then build", deps = { "clean", "build" } }
make.alias("c", "compile")

---------------------------------------------------------------------------- the gate

make.recipe{ name = "check", desc = "cargo check, every target",
             run = function() cargo("check", "--all-targets") end }

make.recipe{ name = "check-all", desc = "cargo check, every target and every feature",
             run = function() cargo("check", "--all-targets", "--all-features") end }

make.recipe{ name = "test", desc = "the suite, every target and every feature",
             run = function() cargo("test", "--all-targets", "--all-features") end }
make.alias("t", "test")

make.recipe{ name = "clippy", desc = "clippy, warnings denied",
             run = function()
               cargo("clippy", "--all-targets", "--all-features", "--", "-D", "warnings")
             end }

make.recipe{ name = "fmt", desc = "format the workspace",
             run = function() cargo("fmt", "--all") end }

make.recipe{ name = "fmt-check", desc = "fail if anything is unformatted",
             run = function() cargo("fmt", "--all", "--", "--check") end }

make.recipe{ name = "rustdoc", desc = "build the docs, warnings denied",
             run = function()
               oslo.env.set("RUSTDOCFLAGS", "-Dwarnings")
               cargo("doc", "--all-features", "--no-deps")
             end }

make.recipe{
  name = "verify",
  desc = "the whole local gate",
  deps = { "fmt-check", "check", "check-all", "test", "clippy", "rustdoc" },
}
make.alias("v", "verify")

---------------------------------------------------------------------------- installing

make.recipe{
  name = "install",
  desc = "put the release binary in $PREFIX/bin",
  deps = { "build" },
  run = function()
    local dest = (os.getenv("DESTDIR") or "") .. PREFIX .. "/bin"
    sh.install("-d", dest)
    sh.install("-m", "755", BIN, dest .. "/" .. NAME)
    print(oslo.ui.style("✓ ", { fg = "green" }) .. dest .. "/" .. NAME)
  end,
}
make.alias("i", "install")

make.recipe{
  name = "uninstall",
  desc = "take it back out of $PREFIX/bin",
  run = function()
    local dest = (os.getenv("DESTDIR") or "") .. PREFIX .. "/bin/" .. NAME
    sh.rm("-f", dest)
    print(oslo.ui.style("✓ ", { fg = "green" }) .. "removed " .. dest)
  end,
}

make.recipe{
  name = "release",
  desc = "cut a version: --type patch | minor | major | M.m.p",
  params = { { "--type", desc = "patch | minor | major | M.m.p" } },
  run = function(a)
    assert(oslo.run{ "sh", "-c", "command -v git-rel", capture = true }.ok,
           "git-rel is not installed; install it first")
    assert(type(a.type) == "string",
           "which release? oslo make release --type patch|minor|major|M.m.p")
    sh.git("rel", a.type)
  end,
}
