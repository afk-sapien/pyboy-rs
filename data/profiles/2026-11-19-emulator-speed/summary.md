Frames per second, 36000 frames from power-on, 9 trials, median with best in brackets.

| Emulator | Red, no picture | Red, picture | Crystal, no picture | Crystal, picture |
| --- | ---: | ---: | ---: | ---: |
| pyboy-rs 0.1.9 | 48,968 (49,323) | 18,145 (18,379) | 36,907 (37,459) | 14,746 (14,916) |
| pyboy-rs 0.1.9, sound_emulated=False | 58,663 (59,395) | 19,560 (19,681) | 42,376 (42,866) | 15,720 (15,798) |
| PyBoy 2.9.0 | 4,491 (4,611) | 3,673 (3,685) | 1,791 (1,792) | 1,773 (1,781) |
| PyBoy 2.9.0, sound_emulated=False | 4,959 (5,007) | 3,948 (3,979) | 1,901 (1,912) | 1,882 (1,895) |
| PyBoy 2.9.0, fast_mode=True | 37,269 (37,781) | 18,307 (18,462) | 23,955 (24,150) | 12,052 (12,243) |
| SameBoy 1.0.5 | 1,066 (1,078) | 997 (1,004) | 1,054 (1,065) | 969 (975) |
| mGBA 0.10.5 | 9,458 (9,539) | 6,689 (6,768) | 7,481 (7,551) | 5,552 (5,593) |
| gambatte-core 5a41a68 | 23,567 (24,079) | 17,915 (18,082) | 18,312 (18,430) | 14,862 (15,030) |

Spread, slowest to best trial, as a share of the median:

- gambatte-core 5a41a68, crystal, no picture: 2.5%
- gambatte-core 5a41a68, crystal, picture: 2.6%
- gambatte-core 5a41a68, red, no picture: 7.6%
- gambatte-core 5a41a68, red, picture: 6.8%
- mGBA 0.10.5, crystal, no picture: 3.5%
- mGBA 0.10.5, crystal, picture: 3.3%
- mGBA 0.10.5, red, no picture: 2.0%
- mGBA 0.10.5, red, picture: 2.7%
- PyBoy 2.9.0, crystal, no picture: 5.5%
- PyBoy 2.9.0, crystal, picture: 1.2%
- PyBoy 2.9.0, red, no picture: 4.5%
- PyBoy 2.9.0, red, picture: 2.5%
- PyBoy 2.9.0, fast_mode=True, crystal, no picture: 5.5%
- PyBoy 2.9.0, fast_mode=True, crystal, picture: 5.7%
- PyBoy 2.9.0, fast_mode=True, red, no picture: 4.0%
- PyBoy 2.9.0, fast_mode=True, red, picture: 1.9%
- PyBoy 2.9.0, sound_emulated=False, crystal, no picture: 2.2%
- PyBoy 2.9.0, sound_emulated=False, crystal, picture: 2.1%
- PyBoy 2.9.0, sound_emulated=False, red, no picture: 2.4%
- PyBoy 2.9.0, sound_emulated=False, red, picture: 8.2%
- pyboy-rs 0.1.9, crystal, no picture: 5.6%
- pyboy-rs 0.1.9, crystal, picture: 2.1%
- pyboy-rs 0.1.9, red, no picture: 2.4%
- pyboy-rs 0.1.9, red, picture: 6.4%
- pyboy-rs 0.1.9, sound_emulated=False, crystal, no picture: 2.1%
- pyboy-rs 0.1.9, sound_emulated=False, crystal, picture: 2.8%
- pyboy-rs 0.1.9, sound_emulated=False, red, no picture: 2.8%
- pyboy-rs 0.1.9, sound_emulated=False, red, picture: 4.4%
- SameBoy 1.0.5, crystal, no picture: 2.4%
- SameBoy 1.0.5, crystal, picture: 2.3%
- SameBoy 1.0.5, red, no picture: 9.8%
- SameBoy 1.0.5, red, picture: 2.2%
