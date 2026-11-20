/* Time one headless SameBoy run, for tools/emulator_speed/run.py.

   Usage: sameboy_speed BOOT_ROM ROM FRAMES PICTURE MODEL

   PICTURE 0 calls GB_set_rendering_disabled. PICTURE 1 draws every frame
   into a memory buffer. MODEL is dmg (DMG-B) or cgb (CGB-E). No sample rate
   is set, so the sound hardware is emulated but no samples are made.

   Turbo mode with no frame skipping and no cap turns off the real-time
   sync, as GB_run_frame does. The debugger input callbacks are cleared so
   the core does not read commands from stdin, and its log goes to stderr.

   One frame is 70,224 cycles of the 4,194,304 Hz clock. GB_run returns the
   time passed in ticks of an 8 MHz clock, so the run lasts FRAMES * 140,448
   ticks. Only the run itself is timed, not loading. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include <Core/gb.h>

static uint32_t pixels[256 * 224];
static unsigned long vblanks;

static uint32_t encode(GB_gameboy_t *gb, uint8_t r, uint8_t g, uint8_t b)
{
    (void)gb;
    return ((uint32_t)r << 16) | ((uint32_t)g << 8) | b;
}

static void vblank(GB_gameboy_t *gb, GB_vblank_type_t type)
{
    (void)gb;
    (void)type;
    vblanks++;
}

static char *no_input(GB_gameboy_t *gb)
{
    (void)gb;
    return NULL;
}

static void log_stderr(GB_gameboy_t *gb, const char *string, GB_log_attributes_t attributes)
{
    (void)gb;
    (void)attributes;
    fputs(string, stderr);
}

int main(int argc, char **argv)
{
    if (argc != 6) {
        fprintf(stderr, "usage: sameboy_speed BOOT_ROM ROM FRAMES PICTURE MODEL\n");
        return 2;
    }
    unsigned long frames = strtoul(argv[3], NULL, 10);
    int picture = strcmp(argv[4], "1") == 0;
    GB_model_t model = strcmp(argv[5], "cgb") == 0 ? GB_MODEL_CGB_E : GB_MODEL_DMG_B;

    GB_gameboy_t *gb = GB_init(GB_alloc(), model);
    GB_set_log_callback(gb, log_stderr);
    GB_set_input_callback(gb, no_input);
    GB_set_async_input_callback(gb, NULL);
    if (GB_load_boot_rom(gb, argv[1]) != 0 || GB_load_rom(gb, argv[2]) != 0) {
        fprintf(stderr, "sameboy_speed: could not load %s or %s\n", argv[1], argv[2]);
        return 1;
    }
    GB_set_rgb_encode_callback(gb, encode);
    GB_set_pixels_output(gb, pixels);
    GB_set_vblank_callback(gb, vblank);
    GB_set_rendering_disabled(gb, !picture);
    GB_set_turbo_mode(gb, true, true);
    GB_set_turbo_cap(gb, 0);

    unsigned long long target = (unsigned long long)frames * 140448ULL;
    unsigned long long ticks = 0;
    struct timespec start, end;
    clock_gettime(CLOCK_MONOTONIC, &start);
    while (ticks < target) {
        ticks += GB_run(gb);
    }
    clock_gettime(CLOCK_MONOTONIC, &end);

    double seconds = (double)(end.tv_sec - start.tv_sec) + (double)(end.tv_nsec - start.tv_nsec) / 1e9;
    printf("sameboy frames=%lu vblanks=%lu seconds=%.6f fps=%.1f\n", frames, vblanks, seconds,
           (double)frames / seconds);
    GB_free(gb);
    GB_dealloc(gb);
    return 0;
}
