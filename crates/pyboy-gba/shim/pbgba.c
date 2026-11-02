// SPDX-License-Identifier: LGPL-3.0-only
// Thin C shim over the mGBA core API so the Rust side only sees an opaque handle.
#include <mgba/core/core.h>
#include <mgba/core/config.h>
#include <mgba/gba/core.h>
#include <mgba/internal/gba/gba.h>
#include <mgba/internal/gba/memory.h>
#include <mgba/internal/gba/video.h>
#include <mgba/internal/arm/isa-inlines.h>
#include <mgba-util/vfs.h>
#include <mgba/core/log.h>
#include <mgba-util/memory.h>
#include <mgba/core/blip_buf.h>

#include <stdlib.h>
#include <string.h>

#define PBGBA_WIDTH 240
#define PBGBA_HEIGHT 160
#define PBGBA_AUDIO_CAPACITY 4096
#define PBGBA_MAX_HOOKS 1024
// Execution hooks are BKPT instructions with this immediate. mGBA's own handler ignores
// immediates of CPU_COMPONENT_MISC_4 and above, and the slot itself points back at PbGba.
#define PBGBA_HOOK_SLOT CPU_COMPONENT_MISC_4

struct PbGbaHook {
	uint32_t address;
	int thumb;
	uint32_t original;
};

struct PbGba {
	// First member, so the CPU component slot can be cast back to PbGba.
	struct mCPUComponent component;
	struct mCore* core;
	color_t* video;
	int16_t* audio;
	size_t audio_frames;
	int audio_enabled;
	void (*chained_bkpt16)(struct ARMCore* cpu, int immediate);
	void (*chained_bkpt32)(struct ARMCore* cpu, int immediate);
	struct PbGbaHook hooks[PBGBA_MAX_HOOKS];
	size_t hook_count;
	// A frame interrupted by a hook keeps its start point so it resumes as one frame.
	int in_frame;
	uint32_t frame_start;
	uint32_t cycle_start;
	int hit;
	uint32_t hit_address;
};

static struct PbGbaHook* _find_hook(struct PbGba* pb, uint32_t address) {
	for (size_t i = 0; i < pb->hook_count; ++i) {
		if (pb->hooks[i].address == address) {
			return &pb->hooks[i];
		}
	}
	return NULL;
}

// Runs inside the BKPT instruction. The pipeline is rewound so the hooked instruction is
// next, with its original opcode already fetched. The BKPT stays in memory, so the hook
// fires again the next time execution reaches it. BKPT takes no cycles, so timing is
// unchanged; stopping the run loop early only splits one batch of cycles in two.
static void _hook_breakpoint(struct ARMCore* cpu, int immediate, int thumb) {
	struct PbGba* pb = (struct PbGba*) cpu->components[PBGBA_HOOK_SLOT];
	if (immediate != PBGBA_HOOK_SLOT) {
		(thumb ? pb->chained_bkpt16 : pb->chained_bkpt32)(cpu, immediate);
		return;
	}
	uint32_t address = _ARMPCAddress(cpu);
	struct PbGbaHook* hook = _find_hook(pb, address);
	if (!hook) {
		return;
	}
	cpu->prefetch[1] = cpu->prefetch[0];
	cpu->prefetch[0] = hook->original;
	cpu->gprs[ARM_PC] -= thumb ? WORD_SIZE_THUMB : WORD_SIZE_ARM;
	pb->hit = 1;
	pb->hit_address = address;
	cpu->nextEvent = cpu->cycles;
}

static void _hook_bkpt16(struct ARMCore* cpu, int immediate) {
	_hook_breakpoint(cpu, immediate, 1);
}

static void _hook_bkpt32(struct ARMCore* cpu, int immediate) {
	_hook_breakpoint(cpu, immediate, 0);
}

static void _quiet_log(struct mLogger* logger, int category, enum mLogLevel level, const char* format, va_list args) {
	(void) logger;
	(void) category;
	(void) level;
	(void) format;
	(void) args;
}

static struct mLogger quiet_logger = { .log = _quiet_log };

struct PbGba* pbgba_create(const uint8_t* rom, size_t rom_len, const uint8_t* bios, size_t bios_len, int skip_bios, unsigned sample_rate) {
	mLogSetDefaultLogger(&quiet_logger);
	struct PbGba* pb = calloc(1, sizeof(*pb));
	if (!pb) {
		return NULL;
	}
	pb->core = GBACoreCreate();
	if (!pb->core || !pb->core->init(pb->core)) {
		free(pb);
		return NULL;
	}
	mCoreInitConfig(pb->core, NULL);
	struct mCoreOptions opts = { .useBios = bios != NULL, .skipBios = skip_bios || bios == NULL, .volume = 0x100 };
	mCoreConfigLoadDefaults(&pb->core->config, &opts);
	mCoreLoadConfig(pb->core);

	pb->video = anonymousMemoryMap(PBGBA_WIDTH * PBGBA_HEIGHT * sizeof(color_t));
	pb->core->setVideoBuffer(pb->core, pb->video, PBGBA_WIDTH);
	pb->audio = calloc(PBGBA_AUDIO_CAPACITY * 2, sizeof(int16_t));
	pb->audio_enabled = sample_rate != 0;

	// VFileMemChunk copies the bytes, so the caller keeps ownership of its buffers.
	struct VFile* rom_vf = VFileMemChunk(rom, rom_len);
	if (!rom_vf || !pb->core->loadROM(pb->core, rom_vf)) {
		pb->core->deinit(pb->core);
		free(pb->audio);
		mappedMemoryFree(pb->video, PBGBA_WIDTH * PBGBA_HEIGHT * sizeof(color_t));
		free(pb);
		return NULL;
	}
	if (bios) {
		struct VFile* bios_vf = VFileMemChunk(bios, bios_len);
		pb->core->loadBIOS(pb->core, bios_vf, 0);
	}
	pb->core->setAudioBufferSize(pb->core, PBGBA_AUDIO_CAPACITY);
	if (sample_rate) {
		blip_set_rates(pb->core->getAudioChannel(pb->core, 0), GBA_ARM7TDMI_FREQUENCY, sample_rate);
		blip_set_rates(pb->core->getAudioChannel(pb->core, 1), GBA_ARM7TDMI_FREQUENCY, sample_rate);
	}
	pb->core->reset(pb->core);

	struct ARMCore* cpu = pb->core->cpu;
	cpu->components[PBGBA_HOOK_SLOT] = &pb->component;
	pb->chained_bkpt16 = cpu->irqh.bkpt16;
	pb->chained_bkpt32 = cpu->irqh.bkpt32;
	cpu->irqh.bkpt16 = _hook_bkpt16;
	cpu->irqh.bkpt32 = _hook_bkpt32;
	return pb;
}

// Hooks live in cartridge ROM only: it is not part of save states, so loading a state
// never drops or duplicates a BKPT. Returns 0 for a bad address or a full table.
int pbgba_hook_set(struct PbGba* pb, uint32_t address, int thumb) {
	uint32_t size = thumb ? WORD_SIZE_THUMB : WORD_SIZE_ARM;
	struct GBA* gba = pb->core->board;
	if (address & (size - 1) || address < BASE_CART0 || address - BASE_CART0 + size > gba->memory.romSize
	    || _find_hook(pb, address) || pb->hook_count == PBGBA_MAX_HOOKS) {
		return 0;
	}
	struct PbGbaHook* hook = &pb->hooks[pb->hook_count++];
	hook->address = address;
	hook->thumb = thumb;
	if (thumb) {
		int16_t old;
		GBAPatch16(pb->core->cpu, address, (int16_t) (0xBE00 | PBGBA_HOOK_SLOT), &old);
		hook->original = (uint16_t) old;
	} else {
		int32_t old;
		GBAPatch32(pb->core->cpu, address, (int32_t) (0xE1200070 | PBGBA_HOOK_SLOT), &old);
		hook->original = (uint32_t) old;
	}
	return 1;
}

int pbgba_hook_clear(struct PbGba* pb, uint32_t address) {
	struct PbGbaHook* hook = _find_hook(pb, address);
	if (!hook) {
		return 0;
	}
	if (hook->thumb) {
		GBAPatch16(pb->core->cpu, address, (int16_t) hook->original, NULL);
	} else {
		GBAPatch32(pb->core->cpu, address, (int32_t) hook->original, NULL);
	}
	*hook = pb->hooks[--pb->hook_count];
	return 1;
}

uint32_t pbgba_hook_hit(struct PbGba* pb) {
	return pb->hit_address;
}

void pbgba_destroy(struct PbGba* pb) {
	if (!pb) {
		return;
	}
	pb->core->deinit(pb->core);
	mappedMemoryFree(pb->video, PBGBA_WIDTH * PBGBA_HEIGHT * sizeof(color_t));
	free(pb->audio);
	free(pb);
}

void pbgba_reset(struct PbGba* pb) {
	pb->in_frame = 0;
	pb->core->reset(pb->core);
}

void pbgba_set_keys(struct PbGba* pb, uint32_t keys) {
	pb->core->setKeys(pb->core, keys);
}

uint32_t pbgba_get_keys(struct PbGba* pb) {
	return pb->core->getKeys(pb->core);
}

static void _drain_audio(struct PbGba* pb) {
	struct blip_t* left = pb->core->getAudioChannel(pb->core, 0);
	struct blip_t* right = pb->core->getAudioChannel(pb->core, 1);
	int avail = blip_samples_avail(left);
	if (!pb->audio_enabled) {
		// Discard so the blip buffers never overflow.
		while (avail > 0) {
			int n = avail > PBGBA_AUDIO_CAPACITY ? PBGBA_AUDIO_CAPACITY : avail;
			blip_read_samples(left, pb->audio, n, 1);
			blip_read_samples(right, pb->audio + 1, n, 1);
			avail -= n;
		}
		pb->audio_frames = 0;
		return;
	}
	size_t room = PBGBA_AUDIO_CAPACITY - pb->audio_frames;
	int keep = (size_t) avail > room ? (int) room : avail;
	int16_t* dest = pb->audio + pb->audio_frames * 2;
	if (keep > 0) {
		blip_read_samples(left, dest, keep, 1);
		blip_read_samples(right, dest + 1, keep, 1);
	}
	avail -= keep;
	while (avail > 0) {
		int16_t scratch[512];
		int n = avail > 256 ? 256 : avail;
		blip_read_samples(left, scratch, n, 1);
		blip_read_samples(right, scratch + 1, n, 1);
		avail -= n;
	}
	avail = keep;
	pb->audio_frames += avail;
}

// Run one frame. With render == 0 the scanline renderer is skipped for this frame,
// while VRAM, palette and register writes are still tracked so later frames are exact.
void pbgba_run_frame(struct PbGba* pb, int render) {
	struct GBA* gba = pb->core->board;
	if (!render) {
		gba->video.frameskipCounter = 1;
	} else {
		gba->video.frameskipCounter = 0;
	}
	pb->core->runFrame(pb->core);
	_drain_audio(pb);
}

// Run one frame like mGBA's runFrame, but return 0 as soon as a hook is reached. Calling
// again resumes the same frame. Returns 1 once the frame is complete.
int pbgba_run_frame_hooked(struct PbGba* pb, int render) {
	struct GBA* gba = pb->core->board;
	if (!pb->in_frame) {
		gba->video.frameskipCounter = render ? 0 : 1;
		pb->audio_frames = 0;
		pb->frame_start = gba->video.frameCounter;
		pb->cycle_start = mTimingCurrentTime(&gba->timing);
		pb->in_frame = 1;
	}
	pb->hit = 0;
	while (gba->video.frameCounter == pb->frame_start
	       && mTimingCurrentTime(&gba->timing) - pb->cycle_start < VIDEO_TOTAL_LENGTH + VIDEO_HORIZONTAL_LENGTH) {
		ARMRunLoop(pb->core->cpu);
		if (pb->hit) {
			return 0;
		}
	}
	pb->in_frame = 0;
	_drain_audio(pb);
	return 1;
}

// Run count frames, rendering only the last one when render_last is set.
void pbgba_run_frames(struct PbGba* pb, uint32_t count, int render_last) {
	uint32_t i = 0;
	if (pb->in_frame && count) {
		// Finish a frame a hook interrupted before its hook was removed.
		while (!pbgba_run_frame_hooked(pb, render_last && count == 1)) {
		}
		i = 1;
	}
	for (; i < count; ++i) {
		// Like PyBoy, the audio buffer holds the samples of the most recent frame.
		pb->audio_frames = 0;
		pbgba_run_frame(pb, render_last && i + 1 == count);
	}
}

uint32_t pbgba_frame_counter(struct PbGba* pb) {
	return pb->core->frameCounter(pb->core);
}

const color_t* pbgba_video(struct PbGba* pb) {
	return pb->video;
}

const int16_t* pbgba_audio(struct PbGba* pb, size_t* frames) {
	*frames = pb->audio_frames;
	return pb->audio;
}

void pbgba_clear_audio(struct PbGba* pb) {
	pb->audio_frames = 0;
}

uint8_t pbgba_read8(struct PbGba* pb, uint32_t address) {
	return pb->core->rawRead8(pb->core, address, -1);
}

void pbgba_read_range(struct PbGba* pb, uint32_t address, uint8_t* out, size_t len) {
	for (size_t i = 0; i < len; ++i) {
		out[i] = pb->core->rawRead8(pb->core, address + (uint32_t) i, -1);
	}
}

void pbgba_write8(struct PbGba* pb, uint32_t address, uint8_t value) {
	pb->core->busWrite8(pb->core, address, value);
}

void pbgba_patch8(struct PbGba* pb, uint32_t address, uint8_t value) {
	pb->core->rawWrite8(pb->core, address, -1, value);
}

// Direct pointer to a memory block by mGBA block id (EWRAM, IWRAM, and so on).
void* pbgba_memory_block(struct PbGba* pb, uint32_t id, size_t* size) {
	return pb->core->getMemoryBlock(pb->core, id, size);
}

size_t pbgba_state_size(struct PbGba* pb) {
	return pb->core->stateSize(pb->core);
}

int pbgba_save_state(struct PbGba* pb, void* out) {
	return pb->core->saveState(pb->core, out);
}

int pbgba_load_state(struct PbGba* pb, const void* in) {
	pb->in_frame = 0;
	return pb->core->loadState(pb->core, in);
}

size_t pbgba_savedata(struct PbGba* pb, void** out) {
	return pb->core->savedataClone(pb->core, out);
}

int pbgba_restore_savedata(struct PbGba* pb, const void* data, size_t len) {
	return pb->core->savedataRestore(pb->core, data, len, true);
}

void pbgba_free(void* ptr) {
	free(ptr);
}

void pbgba_title(struct PbGba* pb, char* out) {
	pb->core->getGameTitle(pb->core, out);
}

void pbgba_code(struct PbGba* pb, char* out) {
	pb->core->getGameCode(pb->core, out);
}

uint32_t pbgba_register(struct PbGba* pb, int index) {
	struct ARMCore* cpu = pb->core->cpu;
	if (index >= 0 && index < 16) {
		return cpu->gprs[index];
	}
	if (index == 16) {
		return cpu->cpsr.packed;
	}
	return 0;
}
