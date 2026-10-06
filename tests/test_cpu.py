import random

import pytest

from pyboy_rs import CPU
from conftest import FlatBus, compare_cpu


@pytest.mark.parametrize("opcode", range(512))
def test_every_opcode_against_python(opcode, oracle):
    rng = random.Random(0xB0_270 + opcode)
    for iteration in range(32):
        memory = bytearray(rng.randbytes(65536))
        state = {name: rng.randrange(256) for name in ("A", "B", "C", "D", "E")}
        state.update(F=(iteration % 16) << 4, HL=rng.randrange(65536),
                     SP=rng.randrange(65536), PC=[0, 0x100, 0xFFFE, 0xFFFF][iteration % 4],
                     interrupts_flag_register=rng.randrange(256),
                     interrupts_enabled_register=rng.randrange(256),
                     interrupt_master_enable=iteration % 2, cycles=100)
        pc = state["PC"]
        memory[pc] = opcode if opcode < 256 else 0xCB
        if opcode >= 256:
            memory[(pc + 1) & 65535] = opcode - 256
        native = CPU(cgb=iteration % 2 == 1)
        native.load_memory(bytes(memory))
        native.set_state(state)
        bus = FlatBus(memory, cgb=iteration % 2 == 1)
        reference = oracle.cpu.CPU(bus)
        for name, value in state.items():
            setattr(reference, name, value)
        reference.fetch_and_execute()
        native.step()
        compare_cpu(native, reference)


@pytest.mark.parametrize("flags", range(32))
@pytest.mark.parametrize("enabled,ime,halted,queued", [(31, 1, 0, 0), (31, 0, 1, 0), (10, 1, 1, 0), (31, 1, 0, 1)])
def test_interrupt_priority_stack_and_halt(flags, enabled, ime, halted, queued, oracle):
    native = CPU()
    reference = oracle.cpu.CPU(FlatBus(bytes(65536)))
    state = dict(PC=65535, SP=0, interrupts_flag_register=flags,
                 interrupts_enabled_register=enabled, interrupt_master_enable=ime,
                 halted=halted, interrupt_queued=queued)
    native.set_state(state)
    for key, value in state.items():
        setattr(reference, key, value)
    assert native.check_interrupts() == reference.check_interrupts()
    compare_cpu(native, reference)


@pytest.mark.parametrize("halted,flags,ime", [(0, 0, 0), (1, 0, 0), (1, 1, 0), (1, 1, 1)])
def test_tick_scheduling(halted, flags, ime, oracle):
    native = CPU()
    reference = oracle.cpu.CPU(FlatBus(bytes(65536)))
    state = dict(PC=256, SP=65534, halted=halted, interrupts_flag_register=flags,
                 interrupts_enabled_register=31, interrupt_master_enable=ime)
    native.set_state(state)
    for key, value in state.items():
        setattr(reference, key, value)
    for budget in (4, 12, 256, 70224):
        reference.tick(budget)
        native.tick(budget)
        compare_cpu(native, reference)


def test_illegal_opcode_cannot_hang_tick():
    cpu = CPU()
    cpu.write(0, 0xD3)
    with pytest.raises(RuntimeError, match="no progress"):
        cpu.tick(4)


def test_validation_is_atomic():
    cpu = CPU()
    before = cpu.state()
    with pytest.raises(ValueError):
        cpu.set_state({"A": 5, "PC": 65536})
    assert cpu.state() == before
    with pytest.raises(ValueError):
        cpu.load_memory(b"too short")
    with pytest.raises(ValueError):
        cpu.tick(-1)
