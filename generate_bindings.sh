#!/bin/zsh

#flipper-z-f7-sdk-XFW-DEV_@D8B43FF
./tools/target/release/generate-bindings ~/gh/Xtreme-Firmware/dist/f7-C/$1/sdk_headers
mv bindings.rs crates/sys/src/bindings.rs
#pushd ../crates
