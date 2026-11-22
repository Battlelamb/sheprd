import unittest

from bench_terminal import measurements, pixel_measurements


class MeasurementsTest(unittest.TestCase):
    def test_pixel_stalls_are_independent_of_window_frame_counts(self):
        records = [{"start": 9}, {"time": 9.5}, {"time": 11}, {"time": 20}, {"end": 23}]
        result = pixel_measurements(records, 10, 22)
        self.assertEqual(result["pixel_changes"], 2)
        self.assertEqual(result["pixel_max_silence_ms"], 9000)

    def test_unchanged_pixels_count_the_whole_interval(self):
        records = [{"start": 9}, {"time": 9.5}, {"end": 23}]
        result = pixel_measurements(records, 10, 22)
        self.assertEqual(result["pixel_changes"], 0)
        self.assertEqual(result["pixel_max_silence_ms"], 12000)

    def test_incomplete_pixel_capture_is_not_a_frozen_terminal(self):
        for records in [[], [{"start": 9}, {"end": 23}],
                        [{"start": 9}, {"time": 11}, {"end": 15}]]:
            with self.assertRaises(RuntimeError):
                pixel_measurements(records, 10, 22)

    def test_a_frozen_phase_counts_the_entire_interval(self):
        result = measurements([], 10, 22, 2, 3)
        self.assertEqual(result["frames"], 0)
        self.assertEqual(result["fps"], 0)
        self.assertIsNone(result["p95_ms"])
        self.assertIsNone(result["max_ms"])
        self.assertEqual(result["max_silence_ms"], 12000)

    def test_one_frame_does_not_hide_a_long_boundary_stall(self):
        result = measurements([(11, 2)], 10, 22, 2, 3)
        self.assertEqual(result["frames"], 1)
        self.assertEqual(result["p95_ms"], 2)
        self.assertEqual(result["max_gap_ms"], 0)
        self.assertEqual(result["max_silence_ms"], 11000)

    def test_internal_gaps_and_frame_cost_are_distinct(self):
        result = measurements([(10.5, 2), (11, 20), (20, 3)], 10, 22, 2, 5)
        self.assertEqual(result["fps"], 0.25)
        self.assertEqual(result["p95_ms"], 20)
        self.assertEqual(result["max_gap_ms"], 9000)
        self.assertEqual(result["max_silence_ms"], 9000)
        self.assertAlmostEqual(result["over_16_7_pct"], 100 / 3)
        self.assertEqual(result["cpu_pct"], 25)


if __name__ == "__main__":
    unittest.main()
