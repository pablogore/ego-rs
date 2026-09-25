#!/bin/sh
# Linker driver for every x86_64 Linux build (see .cargo/config.toml).
#
# Link time dominates the test job: ~114 test binaries, and `cargo check`
# never links at all. mold links that shape much faster than the default ld.
#
# The same .cargo/config.toml is read by the CI runners, by contributors'
# machines, and by the official rust:<version> image that Shipwright's Dagger
# steps (the `lint` job) run in, which ships neither clang nor mold. So mold is
# used when it is installed and the system cc links otherwise, instead of a
# hard `linker = "clang"` + `-fuse-ld=mold` that would break every environment
# without them. The choice happens at link time only: it never changes rustc
# flags, so compiled artifacts and cache keys are identical everywhere.
#
# CI sets EGO_REQUIRE_MOLD=1 so a failed mold install fails the job instead of
# silently falling back to the default linker (rustc's bundled rust-lld).
#
# -fuse-ld=mold goes LAST: since 1.90 rustc links this target through its
# bundled rust-lld by passing its own -fuse-ld=lld, and the driver honours the
# last -fuse-ld it sees. Prepended, mold would be silently ignored. clang is
# preferred as the driver because gcc only understands -fuse-ld=mold from
# version 12 on.

if command -v mold >/dev/null 2>&1; then
    if command -v clang >/dev/null 2>&1; then
        exec clang "$@" -fuse-ld=mold
    fi
    exec cc "$@" -fuse-ld=mold
fi

if [ "${EGO_REQUIRE_MOLD:-}" = "1" ]; then
    echo "cargo-linker.sh: EGO_REQUIRE_MOLD=1 but mold is not on PATH" >&2
    exit 1
fi

exec cc "$@"
