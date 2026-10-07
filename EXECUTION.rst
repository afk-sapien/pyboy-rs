Experimental execution APIs
===========================

These APIs are experimental. They are designed to be called through PokeSim
Core, an optional backend for PokeSim, though they can be used directly. PyBoy RS executes generic controller inputs and Core
interprets Pokemon menus and verifies their effects. No menu policy moves into
the emulator.

Bounded sequences
-----------------

Each step is a pair of a button tuple and a positive frame count. The tuple
specifies the full controller state, including simultaneous buttons. An empty
tuple releases the controller and waits. At most 4096 steps are accepted.

Example through Core::

    emulator.start_sequence([
        (('a',), 8),
        ((), 24),
        (('down',), 8),
        ((), 24),
    ])
    result = emulator.run_sequence(32, render=False, sound=False)
    checkpoint = emulator.checkpoint()
    result = emulator.run_sequence(32, render=False, sound=False)

run_sequence returns frames, reason, completed and a detached progress object.
Reasons are completed, budget, cancelled, paused and stopped. max_frames can
be zero. cancelled is an optional callable checked between frames. Returning
true suspends the sequence without discarding its progress. cancel_sequence
explicitly abandons it and queues releases for the next advanced frame.
Completion also queues releases for the next frame, matching ordinary input
semantics without consuming an unbudgeted extra frame.

Starting requires no pending input and no other active sequence. Use
run_sequence to advance an active sequence. Ordinary tick is rejected until
the sequence finishes or is cancelled. Hooks still execute synchronously and
Core flushes hook-generated pending inputs before the following frame.

Core frame_count reflects completed frames during hooks and cancellation
callbacks, including batched ticks and replay. Checkpoint restoration preserves
the logical Core timeline even when it differs from the backend frame counter.

The first implementation schedules sequences in the Python binding around
native Rust frame execution. It is not a native batch optimization. Frame
budgets, hooks and cancellation take priority over reducing API calls.

Existing menu helpers continue to use ControllerPort.send and choose so the
consumer retains its locks, action log and global frame budget. An adapter
can translate send(button, held_frames, released_frames) into a hold step and
an empty-button wait step. Pass only the remaining consumer budget to
run_sequence and count result['frames']. Reobserve the menu after each
bounded action. Do not replace adaptive menu checks with a long fixed script.

Recording and replay
--------------------

Example::

    emulator.start_recording(max_frames=10000)
    emulator.press('a')
    emulator.tick(8, render=False, sound=False)
    emulator.release('a')
    emulator.tick(24, render=False, sound=False)
    recording = emulator.stop_recording()
    result = another_emulator.replay(recording)
    assert result['verified']

Recordings contain starting and ending checkpoints, the button events
actually applied per frame, rendering and sound flags, and a final hardware
checksum. Replay validates ROM and build identities, advances the recorded
frames, and verifies hardware, supplementary runtime state and ROM overlays.
It then restores final input queues and sequence progress so execution can
continue. A recording is a detached Python data structure containing bytes.
There is no automatic file I/O or unsafe object deserialization API.

The frame limit bounds normal recording growth. A tick that would exceed it
fails before advancing. Recording while paused is rejected. A hook exception
invalidates the recording because the last frame may be incomplete.

Replay requires the same deterministic hooks and caller-owned callback
context. Install matching hook addresses and reset external callback state
before replay. Arbitrary memory edits, changing hooks, external link activity,
and other host side effects are not an input recording format. Divergence
raises an error and leaves the machine at the divergent state for inspection.
Cartridges with a live wall clock are rejected when starting a recording; lock
the clock first with ``lock_clock(at=...)``. A locked clock is part of the
checkpoints and is replayed exactly. Clock changes are refused while recording.
Restoring a checkpoint that has no clock data releases any lock.

Recordings and checkpoints are private local artifacts. They contain game
state and potentially small ROM overlays. Do not commit or distribute them.

Execution checkpoints
---------------------

Core checkpoint format 2 includes its own pending inputs and logical frame
counter plus the backend execution checkpoint. The backend owns delayed
events, sequence progress and native execution state. Runtime state includes
renderer counters, scheduler deadlines, serial output, RTC latches, ROM and
boot ROM overlays, and the backend frame counter. Overlay capture stores
changed bytes only. It does not embed the complete ROM.

restore_checkpoint validates identity and state before replacing execution.
Checkpoints are taken between frames, outside hooks. Python callback objects,
external resources, wall-clock pacing and diagnostic profiling counters are
not serialized. The receiving machine must already have matching hook addresses. Checkpoints
preserve each hook's underlying instruction and restore it while retaining
the receiving machine's callback and context objects.

Legacy save and load methods retain the upstream format-15 hardware bytes.
They do not preserve pending inputs or all runtime fields. Use the execution
checkpoint API for exact resume. Raw loads are rejected while a recording or
sequence is active. Older Core format-1 checkpoints retain their historical
hardware-only backend behavior.

Profiling procedure
-------------------

Example::

    emulator.start_profiling()
    emulator.tick(6000, render=False, sound=False)
    counters = emulator.stop_profiling()

start_profiling resets and enables native counters. profiling_counters returns
a detached snapshot. stop_profiling returns the final snapshot and disables
collection. Native measurements sample approximately one in 64 scheduler
iterations, reporting raw sampled wall nanoseconds for:

* cpu_memory_dma, including CPU execution, bus accesses and DMA
* audio scheduler work
* serial scheduler work
* timers scheduler work
* graphics scheduler work

Hooks report exact callback counts, wall nanoseconds and thread CPU nanoseconds
separately. Component times are sampled wall time, not thread CPU time. Clock reads add
measurement overhead, particularly for very short components. Memory
mapped peripheral work performed inside CPU execution remains in that bucket.
Hook instruction stepping and Python boundary overhead are not included in
the component sample totals. These counters do not form a full wall-time sum.

Run tools/check_execution.py from the Core checkout with the experimental
packages installed to verify all four procedures with the redistributable demo.
It accepts --rom, --state, --frames, --render and --audio for local fixtures.

Profile the same checkpoint and input trace separately from speed benchmarks.
Use counters to find expensive components. Measure final throughput with
profiling and recording off, with the same render and sound settings, and
compare state hashes. Instrumentation is not a speed improvement.

Intermediate replay diagnostics
-------------------------------

start_recording accepts diagnostic_interval and diagnostic_ranges. A nonzero
interval enables hardware, runtime and ROM-overlay hashes plus bounded WRAM
windows. Replay stops at the first failed checkpoint and raises ReplayDivergence
with a details dictionary containing the last verified interval, nearby inputs
and changed bytes. It identifies an interval, not the first differing instruction.
Diagnostics are disabled by default and accept at most 4096 checkpoints.
