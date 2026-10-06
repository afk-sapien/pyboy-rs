import random

import pytest
from pyboy_rs import Sound


@pytest.mark.parametrize("cgb", [False, True])
@pytest.mark.parametrize("sampling", [False, True])
@pytest.mark.parametrize("sample_rate", [24000, 48000])
def test_sound_registers_pcm_and_samples(cgb, sampling, sample_rate, oracle):
    native = Sound(cgb=cgb, sample_rate=sample_rate)
    reference = oracle.sound.Sound(100, True, sample_rate, cgb)
    native.disable_sampling = reference.disable_sampling = not sampling
    rng = random.Random(270)
    cycles = 0
    for step in range(3000):
        if step % 23 == 0:
            native.clear_buffer()
            reference.clear_buffer()
        if step % 59 == 0:
            native.reset_apu_div()
            reference.reset_apu_div()
        if step % 101 == 0:
            native.set(22, 128)
            reference.set(22, 128)
        offset, value = rng.randrange(48), rng.randrange(256)
        native.set(offset, value)
        reference.set(offset, value)
        cycles += rng.randrange(800)
        native.tick(cycles)
        reference.tick(cycles)
        assert [native.get(i) for i in range(48)] == [reference.get(i) for i in range(48)]
        assert native.pcm12() == reference.pcm12()
        assert native.pcm34() == reference.pcm34()
        assert native.samples() == bytes(reference.audiobuffer[:reference.audiobuffer_head])
