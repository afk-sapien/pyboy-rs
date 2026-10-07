from copy import deepcopy

import pytest

from test_execution import machine


@pytest.mark.parametrize('split', [0, 1, 3, 4, 5, 8, 9])
@pytest.mark.parametrize('media', [False, True])
def test_sequence_transfer_at_each_step_boundary_matches_manual_inputs(split, media):
    steps = [(('a', 'right'), 3), ((), 2), (('down',), 4)]
    with machine() as source, machine() as restored, machine() as manual:
        source.tick(90)
        manual.tick(90)
        source.start_sequence(steps)
        source.run_sequence(split, render=media, sound=media)
        checkpoint = source.execution_checkpoint()
        restored.restore_execution(checkpoint)
        source.run_sequence(20, render=media, sound=media)
        restored.run_sequence(20, render=media, sound=media)
        for buttons, frames in steps:
            for button in ('a', 'b', 'start', 'select', 'up', 'down', 'left', 'right'):
                manual.button_release(button)
            for button in buttons:
                manual.button_press(button)
            for _ in range(frames):
                manual.tick(1, render=media, sound=media)
        for button in ('a', 'b', 'start', 'select', 'up', 'down', 'left', 'right'):
            manual.button_release(button)
        for emulator in (source, restored, manual):
            emulator.tick(1, render=media, sound=media)
        assert source.execution_checkpoint() == restored.execution_checkpoint() == manual.execution_checkpoint()


@pytest.mark.parametrize('bank,address', [(-1, 0x80), (0, 0x100), (1, 0x4100)])
def test_hook_originals_survive_repeated_transfer_and_detached_checkpoints(bank, address):
    with machine() as source, machine() as target:
        source.memory[bank, address] = 0x3c
        source.hook_register(bank, address, lambda _: None, None)
        target.hook_register(bank, address, lambda _: None, None)
        checkpoint = source.execution_checkpoint()
        for _ in range(2):
            target.restore_execution(checkpoint)
            assert target.execution_checkpoint() == checkpoint
            target.hook_deregister(bank, address)
            assert target.memory[bank, address] == 0x3c
            target.memory[bank, address] = 0
            target.hook_register(bank, address, lambda _: None, None)
        target.restore_execution(checkpoint)
        checkpoint['hook_opcodes'].clear()
        target.hook_deregister(bank, address)
        assert target.memory[bank, address] == 0x3c


@pytest.mark.parametrize('corrupt', ['state', 'runtime', 'missing_hooks', 'extra_hooks'])
def test_failed_restore_preserves_underlying_hook_instruction(corrupt):
    with machine() as source, machine() as target:
        source.memory[0, 0x100] = 0x3c
        source.hook_register(0, 0x100, lambda _: None, None)
        target.memory[0, 0x100] = 0x04
        target.hook_register(0, 0x100, lambda _: None, None)
        checkpoint = source.execution_checkpoint()
        if corrupt in ('state', 'runtime'):
            checkpoint[corrupt] = b'invalid'
        elif corrupt == 'missing_hooks':
            del checkpoint['hook_opcodes']
        else:
            checkpoint['hooks'].append((0, 0x101))
        before = target.execution_checkpoint()
        with pytest.raises(ValueError):
            target.restore_execution(checkpoint)
        assert target.execution_checkpoint() == before
        target.hook_deregister(0, 0x100)
        assert target.memory[0, 0x100] == 0x04


@pytest.mark.parametrize('corrupt', ['duplicate', 'reverse', 'frame', 'memory_type', 'memory_range',
                                    'state_hash', 'runtime_hash', 'overlay_hash'])
def test_malformed_diagnostics_fail_before_replay_changes_state(corrupt):
    with machine() as emulator:
        emulator.start_recording(4, diagnostic_interval=2, diagnostic_ranges=[(0xc120, 0xc124)])
        emulator.tick(4)
        recording = emulator.stop_recording()
        invalid = deepcopy(recording)
        points = invalid['checkpoints']
        if corrupt == 'duplicate':
            points.append(deepcopy(points[-1]))
        elif corrupt == 'reverse':
            points.reverse()
        elif corrupt == 'frame':
            points[0]['frame'] += 1
        elif corrupt == 'memory_type':
            points[0]['memory'] = [(0xc120, [0, 0, 0, 0])]
        elif corrupt == 'memory_range':
            points[0]['memory'] = [(0xc121, bytes(4))]
        else:
            key = {'state_hash': 'state_sha256', 'runtime_hash': 'runtime_sha256',
                   'overlay_hash': 'overlays_sha256'}[corrupt]
            points[0][key] = 'z' * 64
        emulator.button_press('a')
        before = emulator.execution_checkpoint()
        with pytest.raises(ValueError):
            emulator.replay(invalid)
        assert emulator.execution_checkpoint() == before
        assert emulator.validate_recording(recording)
