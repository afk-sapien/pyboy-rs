from copy import deepcopy
import io

import pytest
from pyboy_rs import PyBoy
from test_machine import DEMO


def machine():
    emulator = PyBoy(DEMO)
    emulator.set_emulation_speed(0)
    return emulator


def hardware(emulator):
    stream = io.BytesIO()
    emulator.save_state(stream)
    return stream.getvalue()


@pytest.mark.parametrize('render,sound', [(False, False), (True, False), (True, True)])
def test_sequences_match_manual_controller_and_resume(render, sound):
    steps = [(('a', 'right'), 3), ((), 2), (('down',), 4)]
    with machine() as seq, machine() as manual, machine() as resumed:
        seq.tick(90)
        manual.tick(90)
        seq.start_sequence(steps)
        result = seq.run_sequence(2, render=render, sound=sound)
        assert result['reason'] == 'budget'
        assert result['frames'] == 2
        assert result['progress']['remaining'] == 1
        checkpoint = seq.execution_checkpoint()
        resumed.restore_execution(checkpoint)
        with pytest.raises(RuntimeError, match='run_sequence'):
            seq.tick()
        assert seq.run_sequence(100, render=render, sound=sound)['frames'] == 7
        assert resumed.run_sequence(100, render=render, sound=sound)['completed']
        for buttons, frames in steps:
            for button in ('up', 'down', 'right', 'left', 'a', 'b', 'select', 'start'):
                manual.button_release(button)
            for button in buttons:
                manual.button_press(button)
            for _ in range(frames):
                manual.tick(1, render=render, sound=sound)
        assert hardware(seq) == hardware(manual) == hardware(resumed)
        assert seq.execution_checkpoint() == resumed.execution_checkpoint()


def test_pause_cancel_invalid_and_zero_budget():
    with machine() as emulator:
        for steps in ([], [(('bogus',), 1)], [(('a', 'a'), 1)], [(('a',), 0)]):
            with pytest.raises(ValueError):
                emulator.start_sequence(steps)
        emulator.start_sequence([(('a',), 5)])
        before = emulator.execution_checkpoint()
        assert emulator.run_sequence(0)['reason'] == 'budget'
        assert emulator.execution_checkpoint() == before
        assert emulator.run_sequence(5, cancelled=lambda: True)['reason'] == 'cancelled'
        emulator.paused = True
        assert emulator.run_sequence(5)['reason'] == 'paused'
        emulator.paused = False
        emulator.run_sequence(2)
        emulator.cancel_sequence()
        assert emulator.sequence_progress is None
        emulator.tick()
        assert emulator.frame_count == 3


def test_scheduled_inputs_restore_with_frame_origin_and_detached_state():
    with machine() as first, machine() as second:
        first.tick(90)
        first.button('a', delay=4)
        checkpoint = first.execution_checkpoint()
        second.restore_execution(checkpoint)
        checkpoint['events'].clear()
        checkpoint['scheduled'].clear()
        for _ in range(6):
            first.tick()
            second.tick()
            assert first.execution_checkpoint() == second.execution_checkpoint()


def test_rejected_checkpoints_are_atomic():
    with machine() as emulator:
        emulator.start_sequence([(('a',), 10)])
        emulator.run_sequence(3)
        before = emulator.execution_checkpoint()
        for field, value in [('state', b'bad'), ('frame', -1), ('frame', 2**64),
                             ('rom_sha256', 'bad'), ('compat', {}), ('clock_lock', {'base': 1}), ('events', [17]),
                             ('scheduled', [(0, 1)]), ('paused', 1),
                             ('sequence', {'steps': [(('a',), 1)], 'index': 3, 'remaining': 1, 'entered': True})]:
            bad = deepcopy(before)
            bad[field] = value
            with pytest.raises((ValueError, OverflowError)):
                emulator.restore_execution(bad)
            assert emulator.execution_checkpoint() == before


@pytest.mark.parametrize('render,sound', [(False, False), (True, False), (True, True)])
def test_record_replay_mixed_ticks_scheduled_inputs_and_sequences(render, sound):
    with machine() as first, machine() as second:
        first.tick(90)
        first.start_recording(100)
        first.button('a', delay=2)
        first.tick(4, render=render, sound=sound)
        first.start_sequence([(('right',), 3), ((), 2)])
        first.run_sequence(4, render=render, sound=sound)
        recording = first.stop_recording()
        result = second.replay(recording)
        assert result['verified']
        assert result['frames'] == 8
        assert first.execution_checkpoint() == second.execution_checkpoint()
        first.run_sequence(1)
        second.run_sequence(1)
        assert first.execution_checkpoint() == second.execution_checkpoint()


def test_release_builds_report_that_profiling_is_not_compiled_in():
    import pyboy_rs
    if pyboy_rs.has_feature('profiling'):
        pytest.skip('profiling build')
    assert not pyboy_rs.HAS_PROFILING
    with machine() as emulator:
        for call in (emulator.start_profiling, emulator.profiling_counters, emulator.stop_profiling):
            with pytest.raises(RuntimeError, match='not compiled'):
                call()
        emulator.tick(2)


def test_hooks_still_execute_and_profile_is_opt_in():
    import pyboy_rs
    profiling = pyboy_rs.has_feature('profiling')
    with machine() as first, machine() as second:
        hits = [0, 0]
        for index, emulator in enumerate((first, second)):
            def callback(_, emulator=emulator, index=index):
                hits[index] += 1
                emulator.memory[0xc123] = 27
                emulator.button('b', delay=2)
            emulator.hook_register(0, 0x100, callback, None)
        first.start_recording()
        if profiling:
            first.start_profiling()
        first.start_sequence([((), 90)])
        first.run_sequence(90)
        profile = first.stop_profiling() if profiling else None
        recording = first.stop_recording()
        second.replay(recording)
        assert hits == [1, 1]
        if not profiling:
            return
        assert profile['hooks']['calls'] == 1
        assert profile['hooks']['wall_ns'] > 0
        assert profile['iterations'] >= profile['samples'] > 0
        assert set(profile['components']) == {'cpu_memory_dma', 'audio', 'serial', 'timers', 'graphics'}
        assert sum(profile['components'].values()) > 0
        assert second.profiling_counters()['iterations'] == 0
        old = first.profiling_counters()
        first.tick(3)
        assert first.profiling_counters()['components'] == old['components']
        first.start_profiling()
        assert first.profiling_counters()['samples'] == 0


def test_recording_limit_and_divergence():
    with machine() as first, machine() as second:
        first.tick(90)
        first.start_recording(1)
        with pytest.raises(ValueError, match='limit'):
            first.tick(2)
        first.tick()
        first.memory[0xc123] = 55
        recording = first.stop_recording()
        with pytest.raises(RuntimeError, match='diverged'):
            second.replay(recording)


def test_hook_failure_invalidates_recording_and_restores_hook():
    with machine() as emulator:
        def fail(_):
            raise LookupError('failure')
        emulator.hook_register(0, 0x100, fail, None)
        emulator.start_recording()
        with pytest.raises(LookupError):
            emulator.tick(90)
        with pytest.raises(RuntimeError, match='interrupted'):
            emulator.stop_recording()
        assert emulator.memory[0, 0x100] == 0xdb


def test_rom_and_boot_overlays_restore_and_replay():
    with machine() as first, machine() as second:
        first.memory[0, 0x2000] = 19
        first.memory[-1, 0x80] = 21
        checkpoint = first.execution_checkpoint()
        second.memory[0, 0x2001] = 20
        second.restore_execution(checkpoint)
        assert first.memory[0, 0x2000:0x2002] == second.memory[0, 0x2000:0x2002]
        assert first.memory[-1, 0x80] == second.memory[-1, 0x80]
        first.start_recording()
        first.tick(2)
        recording = first.stop_recording()
        assert second.replay(recording)['verified']
        assert first.execution_checkpoint() == second.execution_checkpoint()


def test_bad_runtime_and_overlay_do_not_partially_restore():
    with machine() as emulator:
        before = emulator.execution_checkpoint()
        for field, value in [('runtime', b'bad'), ('patches', [(9999, 0, 1)])]:
            invalid = deepcopy(before)
            invalid[field] = value
            with pytest.raises(ValueError):
                emulator.restore_execution(invalid)
            assert emulator.execution_checkpoint() == before


def test_reentrant_cancellation_and_raw_load_are_rejected():
    with machine() as emulator:
        saved = hardware(emulator)
        emulator.start_sequence([((), 2)])
        with pytest.raises(RuntimeError):
            emulator.load_state(io.BytesIO(saved))
        with pytest.raises(RuntimeError):
            emulator.run_sequence(2, cancelled=lambda: emulator.run_sequence(1))
        assert emulator.frame_count == 0
        emulator.cancel_sequence()
        emulator.start_recording()
        emulator.paused = True
        with pytest.raises(RuntimeError, match='Unpause'):
            emulator.tick()
        emulator.paused = False
        emulator.stop_recording()


def test_cgb_checkpoint_and_replay_preserve_render_transitions():
    with PyBoy(DEMO, cgb=True) as first, PyBoy(DEMO, cgb=True) as second:
        for emulator in (first, second):
            emulator.set_emulation_speed(0)
        first.tick(90)
        first.start_recording()
        first.tick(4, render=False, sound=False)
        first.start_sequence([(('a', 'right'), 3), ((), 2)])
        first.run_sequence(2, render=False, sound=False)
        recording = first.stop_recording()
        second.replay(recording)
        first.run_sequence(3, render=True, sound=True)
        second.run_sequence(3, render=True, sound=True)
        assert first.execution_checkpoint() == second.execution_checkpoint()


def test_live_rtc_recording_fails_explicitly():
    rom = bytearray(DEMO.read_bytes())
    rom[0x147] = 0x10
    rom[0x149] = 3
    rom[0x14d] = (-sum(rom[0x134:0x14d]) - 25) & 255
    with PyBoy(io.BytesIO(rom)) as emulator:
        with pytest.raises(ValueError, match='wall clock'):
            emulator.start_recording()


def test_checkpoint_transfers_hook_instruction_and_preserves_callbacks():
    with machine() as first, machine() as second:
        hits = [0, 0]
        values = [[], []]
        machines = (first, second)
        first.memory[0, 0x100] = 0x3c
        def hook(index):
            hits[index] += 1
            machines[index].register_file.A = 10
        def after_instruction(index):
            values[index].append(machines[index].register_file.A)
        for index, emulator in enumerate(machines):
            emulator.hook_register(0, 0x100, hook, index)
            emulator.hook_register(0, 0x101, after_instruction, index)
        checkpoint = first.execution_checkpoint()
        second.restore_execution(checkpoint)
        assert first.execution_checkpoint() == second.execution_checkpoint()
        first.start_recording(90, diagnostic_interval=1)
        first.tick(90)
        recording = first.stop_recording()
        second.tick(90)
        assert hits == [1, 1]
        assert values == [[11], [11]]
        assert first.execution_checkpoint() == second.execution_checkpoint()
        assert second.replay(recording)['verified']
        assert values[1] == [11, 11]
        first.hook_deregister(0, 0x100)
        second.hook_deregister(0, 0x100)
        assert first.memory[0, 0x100] == second.memory[0, 0x100] == 0x3c


@pytest.mark.parametrize('opcodes', [None, [], [(0, 0x100, 256)], [(0, 0x100, 0xdb)],
                                   [(0, 0x101, 0)], [(0, 0x100, True)],
                                   [(0, 0x100, 0), (0, 0x100, 0)]])
def test_invalid_hook_instructions_do_not_mutate_checkpoint(opcodes):
    with machine() as emulator:
        emulator.hook_register(0, 0x100, lambda _: None, None)
        before = emulator.execution_checkpoint()
        invalid = deepcopy(before)
        invalid['hook_opcodes'] = opcodes
        with pytest.raises(ValueError):
            emulator.restore_execution(invalid)
        assert emulator.execution_checkpoint() == before
