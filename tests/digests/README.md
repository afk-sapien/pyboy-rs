# Digests of pyboy-rs's own output

`digests.json` holds SHA-256 digests of what pyboy-rs saves, draws and plays
at fixed points of the demo ROM runs in these tests. The tests compare with
PyBoy 2.7.0 wherever timing cannot change the result: the boot frames with
the LCD off, the state format in both directions, and runs that start from
one shared state and read no LCD register. Elsewhere pyboy-rs follows the
hardware where PyBoy does not ([docs/compatibility.md](../../docs/compatibility.md#divergences)),
so no other emulator says what those bytes must be. The digests catch every
change there, intended or not.

## The rule

A digest changes only for a reason outside pyboy-rs's own output. Any pull
request that changes `digests.json` says in its body which test ROM or
hardware fact justifies the change, for example the Mooneye or Blargg test
that the change makes pass, and lists the digests that moved. "The digests
moved" is not a reason.

## Regenerate

```sh
python -m pytest --update-digests
```

This rewrites `digests.json` from the tests that pass and prints each digest
that moved, was added or was removed. A test that fails keeps its saved
digests. A run over part of the suite updates only the tests it ran. Copy the
printed list into the pull request body beside the reason.
