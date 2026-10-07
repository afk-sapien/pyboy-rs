"""Bounded controller execution and trusted, in-memory replay artifacts.

Hooks remain caller-owned. Exact replay requires the same deterministic hooks
and no out-of-band memory edits, clock changes, or external device activity.
"""

from copy import deepcopy
from functools import lru_cache
import hashlib
import heapq
from importlib.metadata import version
from pathlib import Path
import time

from .diagnostics import ReplayDivergence, compare, snapshot, validate_checkpoints, validate_ranges

BUTTONS = ('up', 'down', 'right', 'left', 'a', 'b', 'select', 'start')


def integer(value, name, minimum=0):
    if type(value) is not int or value < minimum:
        raise ValueError(f'{name} must be an integer >= {minimum}')
    return value


@lru_cache(maxsize=1)
def _build_identity():
    from . import _native
    root = Path(__file__).parent
    digest = hashlib.sha256()
    for name in ('pyboy.py', 'execution.py', 'diagnostics.py'):
        digest.update((root / name).read_bytes())
    return {'version': version('pyboy-rs'),
            'native_sha256': hashlib.sha256(Path(_native.__file__).read_bytes()).hexdigest(),
            'binding_sha256': digest.hexdigest()}


class Execution:
    def _execution_init(self, rom, bootrom, settings):
        self._execution_identity = {
            'rom_sha256': hashlib.sha256(rom).hexdigest(),
            'bootrom_sha256': None if bootrom is None else hashlib.sha256(bootrom).hexdigest(),
            'settings': settings,
        }
        self._sequence = None
        self._running_sequence = False
        self._recording = None
        self._profile_hooks = None

    def _idle(self):
        if self._in_tick or self._running_sequence:
            raise RuntimeError('Operation requires a frame boundary outside a hook')
        if self.stopped:
            raise RuntimeError('The emulator is stopped')

    def start_sequence(self, steps):
        """Each step is (buttons, frames). Empty buttons means release and wait.

        Buttons are the complete controller state for that step. Starting a
        sequence requires an empty input queue. It never advances time itself.
        """
        self._idle()
        if self._sequence is not None or self._events or self._queued_input:
            raise RuntimeError('Finish or cancel existing input before starting a sequence')
        normalized = []
        for buttons, frames in steps:
            buttons = tuple(buttons)
            if any(b not in BUTTONS for b in buttons) or len(set(buttons)) != len(buttons):
                raise ValueError('Expected distinct controller buttons')
            if len(normalized) >= 4096:
                raise ValueError('A sequence supports at most 4096 steps')
            normalized.append((buttons, integer(frames, 'frames', 1)))
        if not normalized:
            raise ValueError('A sequence requires at least one step')
        self._sequence = {'steps': normalized, 'index': 0, 'remaining': normalized[0][1], 'entered': False}

    @property
    def sequence_progress(self):
        return deepcopy(self._sequence)

    def _release_controller(self):
        for button in BUTTONS:
            self.button_release(button)

    def cancel_sequence(self):
        self._idle()
        if self._sequence is not None:
            self._release_controller()
            self._sequence = None

    def run_sequence(self, max_frames, *, render=True, sound=True, cancelled=None, _before_frame=None):
        """Resume up to max_frames, checking cancellation between frames.

        Cancellation retains progress. cancel_sequence explicitly abandons it.
        Ordinary tick is prohibited while a sequence is pending.
        """
        self._idle()
        integer(max_frames, 'max_frames')
        if cancelled is not None and not callable(cancelled):
            raise TypeError('cancelled must be callable')
        start = self.frame_count
        reason = 'completed'
        self._running_sequence = True
        try:
            while self._sequence is not None:
                if self.stopped or self._quitting:
                    reason = 'stopped'
                    break
                if self.paused:
                    reason = 'paused'
                    break
                if cancelled is not None and cancelled():
                    reason = 'cancelled'
                    break
                if self.frame_count - start >= max_frames:
                    reason = 'budget'
                    break
                current = self._sequence
                if _before_frame is not None:
                    _before_frame()
                if not current['entered']:
                    self._release_controller()
                    for button in current['steps'][current['index']][0]:
                        self.button_press(button)
                    current['entered'] = True
                before = self.frame_count
                try:
                    self.tick(1, render=render, sound=sound)
                finally:
                    current['remaining'] -= self.frame_count - before
                    if current['remaining'] == 0:
                        current['index'] += 1
                        current['entered'] = False
                        if current['index'] == len(current['steps']):
                            self._release_controller()
                            self._sequence = None
                        else:
                            current['remaining'] = current['steps'][current['index']][1]
        finally:
            self._running_sequence = False
        return {'frames': self.frame_count - start, 'reason': reason,
                'completed': self._sequence is None, 'progress': self.sequence_progress}

    def execution_checkpoint(self):
        """Detached hardware and input state. Hook functions are not serialized."""
        self._idle()
        return deepcopy({'format': 1, 'build': _build_identity(), **self._execution_identity,
                         'state': self._machine.save_state(), 'runtime': self._machine.execution_runtime(),
                         'frame': self.frame_count,
                         'events': self._events, 'scheduled': self._queued_input,
                         'sequence': self._sequence, 'paused': self.paused,
                         'quitting': self._quitting, 'hooks': sorted(self._hooks),
                         'hook_opcodes': [(bank, address, self._hooks[bank, address][0])
                                          for bank, address in sorted(self._hooks)],
                         'patches': [(bank, address, self.memory[bank, address])
                                     for (bank, address), original in sorted(self.memory._rom_original.items())
                                     if self.memory[bank, address] != original]})

    def _validate_execution(self, checkpoint):
        checkpoint = deepcopy(checkpoint)
        if checkpoint.get('format') != 1 or checkpoint.get('build') != _build_identity():
            raise ValueError('Execution checkpoint requires the original build')
        if any(checkpoint.get(k) != v for k, v in self._execution_identity.items()):
            raise ValueError('Execution checkpoint ROM or settings mismatch')
        if checkpoint.get('hooks') != sorted(self._hooks):
            raise ValueError('Install the same hooks before restoring')
        opcodes = checkpoint.get('hook_opcodes')
        if not isinstance(opcodes, list):
            raise ValueError('Missing hook instructions')
        locations = []
        for bank, address, original in opcodes:
            if (type(bank) is not int or type(address) is not int or type(original) is not int
                    or not 0 <= original <= 255 or original == 0xdb):
                raise ValueError('Invalid hook instruction')
            locations.append((bank, address))
        if locations != sorted(self._hooks):
            raise ValueError('Hook instructions must match installed hooks exactly')
        keys = set()
        for bank, address, value in checkpoint['patches']:
            if (type(bank) is not int or type(address) is not int or type(value) is not int
                    or not 0 <= address < 16384 or not 0 <= value <= 255 or (bank, address) in keys):
                raise ValueError('Invalid ROM overlay')
            self.memory[bank, address]
            keys.add((bank, address))
        frame = integer(checkpoint['frame'], 'frame')
        if frame > 2**64 - 1 or not isinstance(checkpoint['state'], bytes):
            raise ValueError('Invalid hardware state or frame')
        for event in checkpoint['events']:
            if type(event) is not int or not 0 <= event <= 16:
                raise ValueError('Invalid input event')
        for deadline, event in checkpoint['scheduled']:
            integer(deadline, 'deadline', frame + 1)
            if type(event) is not int or not 0 <= event <= 16:
                raise ValueError('Invalid scheduled event')
        for flag in ('paused', 'quitting'):
            if type(checkpoint[flag]) is not bool:
                raise ValueError('Invalid execution flag')
        sequence = checkpoint['sequence']
        if sequence is not None:
            steps = sequence['steps']
            if not steps:
                raise ValueError('Empty sequence')
            for buttons, frames in steps:
                if any(b not in BUTTONS for b in buttons) or len(set(buttons)) != len(buttons):
                    raise ValueError('Invalid sequence buttons')
                integer(frames, 'frames', 1)
            index = integer(sequence['index'], 'index')
            if index >= len(steps):
                raise ValueError('Invalid sequence index')
            remaining = integer(sequence['remaining'], 'remaining', 1)
            if remaining > steps[index][1] or type(sequence['entered']) is not bool:
                raise ValueError('Invalid sequence progress')
            if not sequence['entered'] and remaining != steps[index][1]:
                raise ValueError('Unstarted step has partial progress')
        return checkpoint

    def restore_execution(self, checkpoint):
        self._idle()
        if self._recording is not None:
            raise RuntimeError('Stop recording before restoring')
        checkpoint = self._validate_execution(checkpoint)
        self._machine.restore_execution(checkpoint['state'], checkpoint['runtime'], checkpoint['frame'])
        for key, original in list(self.memory._rom_original.items()):
            self.memory[key] = original
        for bank, address, value in checkpoint['patches']:
            self.memory[bank, address] = value
        for bank, address, original in checkpoint['hook_opcodes']:
            _, callback, context = self._hooks[bank, address]
            self._hooks[bank, address] = (original, callback, context)
        self._events = list(checkpoint['events'])
        self._queued_input = list(checkpoint['scheduled'])
        heapq.heapify(self._queued_input)
        self._sequence = checkpoint['sequence']
        self.paused = checkpoint['paused']
        self._quitting = checkpoint['quitting']
        self._refresh()

    def start_recording(self, max_frames=100000, *, diagnostic_interval=0, diagnostic_ranges=((0xc000, 0xe000),)):
        integer(diagnostic_interval, 'diagnostic_interval')
        ranges = validate_ranges(diagnostic_ranges)
        integer(max_frames, 'max_frames', 1)
        if diagnostic_interval and (max_frames + diagnostic_interval - 1) // diagnostic_interval > 4096:
            raise ValueError('At most 4096 diagnostic checkpoints are supported')
        self._idle()
        integer(max_frames, 'max_frames', 1)
        if self._recording is not None:
            raise RuntimeError('Already recording')
        if self._machine.has_live_rtc():
            raise ValueError('Deterministic recording requires a cartridge without a live wall clock')
        self._recording = {'format': 1, 'initial': self.execution_checkpoint(),
                           'frames': [], 'limit': max_frames, 'failed': False,
                           'diagnostic_interval': diagnostic_interval, 'diagnostic_ranges': ranges, 'checkpoints': []}

    def _record_frame(self, events, render, sound):
        recording = self._recording
        recording['frames'].append({'events': events, 'render': render, 'sound': sound})
        count = len(recording['frames'])
        interval = recording['diagnostic_interval']
        if interval and count % interval == 0:
            recording['checkpoints'].append(snapshot(self, count, recording['diagnostic_ranges']))

    def stop_recording(self):
        self._idle()
        if self._recording is None:
            raise RuntimeError('No active recording')
        recording, self._recording = self._recording, None
        if recording['failed']:
            raise RuntimeError('Recording interrupted within a frame and cannot be replayed')
        count = len(recording['frames'])
        if recording['diagnostic_interval'] and count and (
                not recording['checkpoints'] or recording['checkpoints'][-1]['offset'] != count):
            recording['checkpoints'].append(snapshot(self, count, recording['diagnostic_ranges']))
        recording['final'] = self.execution_checkpoint()
        recording['final_sha256'] = hashlib.sha256(recording['final']['state']).hexdigest()
        return recording

    def _checked_recording(self, recording):
        recording = deepcopy(recording)
        if recording.get('format') != 1 or recording.get('failed'):
            raise ValueError('Invalid recording')
        initial = self._validate_execution(recording['initial'])
        final = self._validate_execution(recording['final'])
        if final['frame'] - initial['frame'] != len(recording['frames']):
            raise ValueError('Recording frame count mismatch')
        for row in recording['frames']:
            for event in row['events']:
                if type(event) is not int or not 0 <= event <= 16:
                    raise ValueError('Invalid recorded input')
            if any(type(row[k]) is not bool for k in ('render', 'sound')):
                raise ValueError('Invalid recorded media flags')
        if hashlib.sha256(final['state']).hexdigest() != recording['final_sha256']:
            raise ValueError('Recording final checksum mismatch')
        checkpoints, ranges = validate_checkpoints(recording)
        return recording, initial, final, checkpoints, ranges

    def validate_recording(self, recording):
        self._idle()
        self._checked_recording(recording)
        return True

    def replay(self, recording):
        """Replay trusted local data, rejecting build mismatch and divergence.

        Captures applied inputs and media flags at each emulated frame, including
        inputs generated by sequences. External hook context must be reset by
        the caller. This does not serialize arbitrary Python callback state.
        """
        self._idle()
        if self._recording is not None:
            raise RuntimeError('Stop recording before replay')
        recording, initial, final, checkpoints, ranges = self._checked_recording(recording)
        self.restore_execution(initial)
        self._sequence = None
        last_verified = 0
        for offset, row in enumerate(recording['frames'], 1):
            self.paused = False
            self._events = list(row['events'])
            self._queued_input = []
            self.tick(1, render=row['render'], sound=row['sound'])
            if offset in checkpoints:
                compare(recording, checkpoints[offset], snapshot(self, offset, ranges), last_verified)
                last_verified = offset
        actual = hashlib.sha256(self._machine.save_state()).hexdigest()
        patches = self.execution_checkpoint()['patches']
        if actual != recording['final_sha256'] or self._machine.execution_runtime() != final['runtime'] or patches != final['patches']:
            raise ReplayDivergence({'last_verified_offset': last_verified,
                                    'first_failed_offset': len(recording['frames']),
                                    'absolute_frame': self.frame_count,
                                    'resolution_frames': len(recording['frames']) - last_verified,
                                    'expected_state_sha256': recording['final_sha256'], 'actual_state_sha256': actual,
                                    'memory_differences': [], 'nearby_inputs': [],
                                    'reason': 'Final hardware, runtime or ROM overlay mismatch'})
        self.restore_execution(final)
        return {'frames': len(recording['frames']), 'verified': True, 'state_sha256': actual}

    def start_profiling(self):
        self._idle()
        self._machine.profile_start()
        self._profile_hooks = {'calls': 0, 'wall_ns': 0, 'thread_cpu_ns': 0}

    def profiling_counters(self):
        self._idle()
        counters = self._machine.profile_snapshot()
        counters['hooks'] = dict(self._profile_hooks or {'calls': 0, 'wall_ns': 0, 'thread_cpu_ns': 0})
        return counters

    def stop_profiling(self):
        counters = self.profiling_counters()
        self._machine.profile_stop()
        self._profile_hooks = None
        return counters

    def _invoke_callback(self, callback, context):
        if self._profile_hooks is None:
            return callback(context)
        wall, cpu = time.perf_counter_ns(), time.thread_time_ns()
        try:
            return callback(context)
        finally:
            self._profile_hooks['calls'] += 1
            self._profile_hooks['wall_ns'] += time.perf_counter_ns() - wall
            self._profile_hooks['thread_cpu_ns'] += time.thread_time_ns() - cpu
