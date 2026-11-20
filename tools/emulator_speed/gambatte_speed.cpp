// Time one headless Gambatte run, for tools/emulator_speed/run.py.
//
// Usage: gambatte_speed ROM FRAMES PICTURE
//
// The ROM boots with the NO_BIOS load flag, and READONLY_SAV keeps the run
// from writing a save file. PICTURE 0 sets the NO_SOUND and NO_VIDEO speedup
// flags and passes no video buffer. PICTURE 1 sets NO_SOUND and draws every
// frame into a memory buffer. NO_PPU_CALL is not used, because gambatte.h
// says it breaks the LCD interrupt.
//
// runFor counts time in samples of a 2,097,152 Hz clock, 35,112 per frame,
// so the run lasts FRAMES * 35,112 samples. Only the run itself is timed.

#include <gambatte.h>

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

int main(int argc, char **argv) {
	if (argc != 4) {
		std::fprintf(stderr, "usage: gambatte_speed ROM FRAMES PICTURE\n");
		return 2;
	}
	unsigned long long frames = std::strtoull(argv[2], 0, 10);
	bool picture = std::strcmp(argv[3], "1") == 0;

	gambatte::GB gb;
	if (gb.load(argv[1], gambatte::GB::NO_BIOS | gambatte::GB::READONLY_SAV) != gambatte::LOADRES_OK) {
		std::fprintf(stderr, "gambatte_speed: could not load %s\n", argv[1]);
		return 1;
	}
	unsigned flags = gambatte::GB::NO_SOUND;
	if (!picture)
		flags |= gambatte::GB::NO_VIDEO;
	gb.setSpeedupFlags(flags);

	std::vector<gambatte::uint_least32_t> video(160 * 144);
	std::vector<gambatte::uint_least32_t> audio(35112 + 2064);
	unsigned long long target = frames * 35112ULL;
	unsigned long long done = 0;
	unsigned long long drawn = 0;

	std::chrono::steady_clock::time_point start = std::chrono::steady_clock::now();
	while (done < target) {
		std::size_t samples = 35112;
		if (gb.runFor(picture ? video.data() : 0, 160, audio.data(), samples) >= 0)
			drawn++;
		done += samples;
	}
	std::chrono::steady_clock::time_point end = std::chrono::steady_clock::now();

	double seconds = std::chrono::duration<double>(end - start).count();
	std::printf("gambatte frames=%llu drawn=%llu seconds=%.6f fps=%.1f\n", frames, drawn, seconds,
	            static_cast<double>(frames) / seconds);
	return 0;
}
