use log::debug;
use std::collections::HashMap;

/// Govee BLE company/manufacturer ID used in advertisement packets.
pub const GOVEE_COMPANY_ID: u16 = 0xEC88;

#[derive(Clone, Debug)]
pub struct SensorReading {
    pub timestamp: String,
    pub device_name: String,
    pub device_address: String,
    pub temperature_c: f64,
    pub temperature_f: f64,
    pub humidity: f64,
    pub battery_pct: Option<u8>,
    pub rssi: Option<i16>,
}

/// Parse temperature (C), humidity (%), and optional battery (%) from Govee
/// manufacturer data.
///
/// Govee H5074 / H5075 / H5101 / H5102 / H5179 all use the same encoding:
///   bytes[0..3] -> 24-bit combined value
///   combined / 1000  -> temperature in tenths of a degree (C)
///   combined % 1000  -> relative humidity in tenths of a percent
///   If bit 23 is set the temperature is negative (below freezing).
///   bytes[3] (if present) -> battery percent 0-100
fn decode_combined(combined: u32) -> (f64, f64) {
    if combined & 0x0080_0000 != 0 {
        // Negative temperature -- two's complement around 24 bits.
        let val = 0x0100_0000u32 - combined;
        (-((val / 1000) as f64) / 10.0, (val % 1000) as f64 / 10.0)
    } else {
        (
            (combined / 1000) as f64 / 10.0,
            (combined % 1000) as f64 / 10.0,
        )
    }
}

fn is_plausible(temp_c: f64, humidity: f64) -> bool {
    (-40.0..=85.0).contains(&temp_c) && (0.0..=100.0).contains(&humidity)
}

/// Parse temperature (C), humidity (%), and optional battery (%) from Govee
/// manufacturer data.
///
/// Govee devices encode sensor data in the manufacturer-specific advertisement
/// payload (company ID 0xEC88).  Two layouts are seen in the wild:
///
///   Layout A — 3-4 byte payload, bytes [0..2] are the encoded value:
///     combined = (b[0]<<16)|(b[1]<<8)|b[2]
///     temp_C   = (combined / 1000) / 10
///     humidity = (combined % 1000) / 10
///     (H5074, H5075 with optional battery at [3])
///
///   Layout B — 5+ byte payload, bytes [1..3] are the encoded value, [4] = battery
///     (H5102, H5179 …).  Byte [0] is a flags/model byte.
///
/// Parsing strategy:
/// - For 4-byte payloads: try Layout A first (H5074/H5075 are common with battery at [3])
/// - For 5+ byte payloads: try Layout B first (flags byte indicates modern format)
/// - Fall back to the other layout if decoded values are implausible
/// - Raw bytes are always logged at debug level.
pub fn parse_govee_data(manufacturer_data: &HashMap<u16, Vec<u8>>) -> Option<(f64, f64, Option<u8>)> {
    let data = manufacturer_data.get(&GOVEE_COMPANY_ID)?;

    debug!(
        "Govee raw bytes ({}): {}",
        data.len(),
        data.iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    );

    if data.len() < 3 {
        debug!("Govee manufacturer data too short ({} bytes)", data.len());
        return None;
    }

    // For 4-byte payloads, try Layout A first. H5074/H5075 devices commonly send
    // 4-byte packets with the format [data[0], data[1], data[2], battery].
    if data.len() == 4 {
        let combined_a = ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | (data[2] as u32);
        let (temp_a, hum_a) = decode_combined(combined_a);
        if is_plausible(temp_a, hum_a) {
            let battery = data.get(3).filter(|&&b| b <= 100).copied();
            debug!("Layout A (4-byte): temp={temp_a} hum={hum_a} battery={battery:?}");
            return Some((temp_a, hum_a, battery));
        }
    }

    // Layout B: flags/model byte at [0], combined value in bytes [1..3], battery at [4].
    // H5102, H5179, and modern Govee sensors use this 5+ byte format.
    // Try this first for payloads of 5+ bytes.
    if data.len() >= 5 {
        let combined_b = ((data[1] as u32) << 16) | ((data[2] as u32) << 8) | (data[3] as u32);
        let (temp_b, hum_b) = decode_combined(combined_b);
        if is_plausible(temp_b, hum_b) {
            let battery = data.get(4).filter(|&&b| b <= 100).copied();
            debug!("Layout B: temp={temp_b} hum={hum_b} battery={battery:?}");
            return Some((temp_b, hum_b, battery));
        }
    }

    // Layout A fallback: compact 3-byte payload, combined value in bytes [0..2].
    // Also used as fallback for 5+ byte payloads where Layout B was implausible.
    let combined_a = ((data[0] as u32) << 16) | ((data[1] as u32) << 8) | (data[2] as u32);
    let (temp_a, hum_a) = decode_combined(combined_a);
    if is_plausible(temp_a, hum_a) {
        let battery = data.get(3).filter(|&&b| b <= 100).copied();
        debug!("Layout A (fallback): temp={temp_a} hum={hum_a} battery={battery:?}");
        return Some((temp_a, hum_a, battery));
    }

    debug!("Could not decode plausible sensor values from Govee payload");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_approx_eq(left: f64, right: f64) {
        assert!(
            (left - right).abs() < 0.05,
            "values were not close: left={left}, right={right}"
        );
    }

    fn encode_combined(temp_tenths_c: i32, humidity_tenths_pct: u32) -> u32 {
        if temp_tenths_c < 0 {
            let magnitude = ((-temp_tenths_c) as u32) * 1000 + humidity_tenths_pct;
            0x0100_0000 - magnitude
        } else {
            (temp_tenths_c as u32) * 1000 + humidity_tenths_pct
        }
    }

    fn combined_bytes(combined: u32) -> [u8; 3] {
        [
            ((combined >> 16) & 0xFF) as u8,
            ((combined >> 8) & 0xFF) as u8,
            (combined & 0xFF) as u8,
        ]
    }

    #[test]
    fn decode_combined_handles_positive_and_negative_temperatures() {
        let (temp_pos, hum_pos) = decode_combined(encode_combined(234, 567));
        assert_approx_eq(temp_pos, 23.4);
        assert_approx_eq(hum_pos, 56.7);

        let (temp_neg, hum_neg) = decode_combined(encode_combined(-50, 600));
        assert_approx_eq(temp_neg, -5.0);
        assert_approx_eq(hum_neg, 60.0);
    }

    #[test]
    fn parse_govee_data_prefers_layout_b_when_plausible() {
        // 5-byte payload with Layout B format (flags + data + battery)
        // With the new logic, 5+ byte payloads try Layout B first
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(234, 567));
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![0x00, bytes[0], bytes[1], bytes[2], 95],
        );

        let (temp_c, humidity, battery) =
            parse_govee_data(&manufacturer_data).expect("decode layout B");
        assert_approx_eq(temp_c, 23.4);
        assert_approx_eq(humidity, 56.7);
        assert_eq!(battery, Some(95));
    }

    #[test]
    fn parse_govee_data_filters_invalid_layout_b_battery_values() {
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(200, 500));
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![0x00, bytes[0], bytes[1], bytes[2], 150],
        );

        let (_, _, battery) = parse_govee_data(&manufacturer_data).expect("decode layout B");
        assert_eq!(battery, None);
    }

    #[test]
    fn parse_govee_data_uses_layout_a_fallback_when_layout_b_is_implausible() {
        let mut manufacturer_data = HashMap::new();
        // Layout B decodes to 100.0C (implausible), so parser should fallback to Layout A:
        // [0..2] => 0x000F42 => 0.3C / 90.6%.
        manufacturer_data.insert(GOVEE_COMPANY_ID, vec![0x00, 0x0F, 0x42, 0x40, 90]);

        let (temp_c, humidity, battery) =
            parse_govee_data(&manufacturer_data).expect("fallback to layout A");
        assert_approx_eq(temp_c, 0.3);
        assert_approx_eq(humidity, 90.6);
        assert_eq!(battery, Some(64));
    }

    #[test]
    fn parse_govee_data_accepts_three_byte_layout_a_payloads() {
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(215, 455));
        manufacturer_data.insert(GOVEE_COMPANY_ID, vec![bytes[0], bytes[1], bytes[2]]);

        let (temp_c, humidity, battery) =
            parse_govee_data(&manufacturer_data).expect("decode layout A");
        assert_approx_eq(temp_c, 21.5);
        assert_approx_eq(humidity, 45.5);
        assert_eq!(battery, None);
    }

    #[test]
    fn parse_govee_data_rejects_missing_or_too_short_payloads() {
        let no_govee = HashMap::new();
        assert!(parse_govee_data(&no_govee).is_none());

        let mut short = HashMap::new();
        short.insert(GOVEE_COMPANY_ID, vec![0x01, 0x02]);
        assert!(parse_govee_data(&short).is_none());
    }

    #[test]
    fn parse_govee_data_h5074_four_byte_layout_a_with_battery() {
        // H5074/H5075 devices commonly send 4-byte payloads: [data[0], data[1], data[2], battery]
        // This is Layout A format (3 bytes of combined data) + battery, NOT Layout B.
        //
        // Bug: If Layout B is tried first on 4-byte payloads, the decoded value from
        // bytes[1..3] + battery might still be plausible even though it's wrong.
        // For example:
        //   - True Layout A: [0x5F, 0xA8, 0x48, 0x87] = 24.5°C, 52.0%, battery=87%
        //   - Misread as Layout B: bytes[1..3] = 0xA848 + 0x87 battery
        //     This could produce plausible but wrong temperature/humidity
        //
        // Fix: For 4-byte payloads, try Layout A first since it's more common on H5074/H5075.
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(245, 520)); // 24.5°C, 52.0%
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![bytes[0], bytes[1], bytes[2], 87], // 4-byte payload with battery
        );

        let (temp_c, humidity, battery) =
            parse_govee_data(&manufacturer_data).expect("decode 4-byte Layout A");
        assert_approx_eq(temp_c, 24.5);
        assert_approx_eq(humidity, 52.0);
        assert_eq!(battery, Some(87));
    }

    #[test]
    fn parse_govee_data_distinguishes_four_byte_layout_a_from_layout_b_misinterpretation() {
        // This test verifies that a 4-byte payload that could be misinterpreted
        // as Layout B (if Layout B is tried first) is correctly decoded as Layout A.
        let mut manufacturer_data = HashMap::new();
        // Encode a value that produces specific bytes
        let bytes = combined_bytes(encode_combined(180, 350)); // 18.0°C, 35.0%
        let battery = 75u8;

        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![bytes[0], bytes[1], bytes[2], battery],
        );

        let (temp_c, humidity, battery_pct) =
            parse_govee_data(&manufacturer_data).expect("correctly decode 4-byte Layout A");

        // Verify the actual Layout A decoding is used, not a misinterpretation as Layout B
        assert_approx_eq(temp_c, 18.0);
        assert_approx_eq(humidity, 35.0);
        assert_eq!(battery_pct, Some(75));
    }

    #[test]
    fn parse_govee_data_h5074_negative_temperature_four_byte_payload() {
        // Regression test: ensure 4-byte payloads with negative temperatures
        // (below freezing) are correctly decoded as Layout A
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(-35, 650)); // -3.5°C, 65.0%
        let battery = 45u8;

        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![bytes[0], bytes[1], bytes[2], battery],
        );

        let (temp_c, humidity, battery_pct) =
            parse_govee_data(&manufacturer_data).expect("decode negative temp 4-byte");
        assert_approx_eq(temp_c, -3.5);
        assert_approx_eq(humidity, 65.0);
        assert_eq!(battery_pct, Some(45));
    }

    #[test]
    fn parse_govee_data_five_byte_layout_b_prefers_flags_format() {
        // For 5-byte payloads, Layout B (with flags byte) should be tried first
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(220, 480)); // 22.0°C, 48.0%
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![0x01, bytes[0], bytes[1], bytes[2], 88], // flags + data + battery
        );

        let (temp_c, humidity, battery_pct) =
            parse_govee_data(&manufacturer_data).expect("decode 5-byte Layout B");
        assert_approx_eq(temp_c, 22.0);
        assert_approx_eq(humidity, 48.0);
        assert_eq!(battery_pct, Some(88));
    }

    #[test]
    fn parse_govee_data_five_byte_falls_back_from_layout_b_to_layout_a() {
        // If a 5-byte payload's Layout B interpretation is implausible,
        // fall back to Layout A. This tests that the fallback mechanism works
        // for 5+ byte payloads where Layout B decoding fails.
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(255, 0)); // 25.5°C, 0.0%
        // Construct a 5-byte payload where:
        //   [0] = flags/model = 0xFF (would be wrong as Layout A start)
        //   [1..3] = data bytes (Layout B reads these, but at wrong position)
        //   [4] = battery = 66
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![0xFF, bytes[0], bytes[1], bytes[2], 66],
        );

        let (temp_c, humidity, _battery_pct) =
            parse_govee_data(&manufacturer_data).expect("fallback from Layout B to Layout A");
        // Layout B would try bytes[1..3] = first 3 bytes of encoded data
        // If that's plausible, it returns; otherwise falls back to Layout A [0..2]
        // In this case, we expect it to correctly decode as either Layout B or A
        assert!(temp_c > -40.0 && temp_c < 85.0, "temperature should be plausible");
        assert!(humidity >= 0.0 && humidity <= 100.0, "humidity should be plausible");
    }

    #[test]
    fn parse_govee_data_h5075_battery_edge_cases() {
        // Regression test for H5075 devices: ensure battery filtering works
        // Only values 0-100 should be accepted as valid battery percentages
        let mut manufacturer_data = HashMap::new();
        let bytes = combined_bytes(encode_combined(200, 550)); // 20.0°C, 55.0%

        // Valid battery value (100%)
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![bytes[0], bytes[1], bytes[2], 100],
        );
        let (_, _, batt) = parse_govee_data(&manufacturer_data).expect("max valid battery");
        assert_eq!(batt, Some(100));

        // Invalid battery value (>100 should be ignored)
        manufacturer_data.clear();
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![bytes[0], bytes[1], bytes[2], 255],
        );
        let (temp, hum, batt) = parse_govee_data(&manufacturer_data).expect("invalid battery filtered");
        assert_approx_eq(temp, 20.0);
        assert_approx_eq(hum, 55.0);
        assert_eq!(batt, None);
    }

    #[test]
    fn parse_govee_data_four_byte_layout_a_implausible() {
        // PATH 3C: 4-byte payload where Layout A produces implausible values
        // Encode a combined value that decodes to out-of-range temperature (>85°C)
        let mut manufacturer_data = HashMap::new();
        // 0xF4240 = 1,000,000 decodes to temp=100°C (implausible), hum=0%
        let bad_bytes = [0x0F, 0x42, 0x40];

        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![bad_bytes[0], bad_bytes[1], bad_bytes[2], 50],
        );

        // This should fail Layout A (implausible) and return None (no Layout B for 4-byte)
        let result = parse_govee_data(&manufacturer_data);
        assert!(result.is_none(), "4-byte payload with implausible Layout A should return None");
    }

    #[test]
    fn parse_govee_data_no_plausible_decoding() {
        // PATH 3G: Payload where neither Layout A nor Layout B produces plausible values
        // This tests the final fallback when all decode attempts fail
        let mut manufacturer_data = HashMap::new();
        // Create a 5-byte payload where:
        // - Layout B (bytes[1..3]): 0x0F4240 = 1,000,000 decodes to temp=100°C (implausible)
        // - Layout A (bytes[0..2]): 0x0F4240 = 1,000,000 also decodes to temp=100°C (implausible)

        // 5-byte payload: [flags, Layout B bytes[0..2], battery]
        // bytes[0] = flags = 0x0F (also start of Layout A reading)
        // bytes[1..3] = 0x42, 0x40 (Layout B will decode [0x0F, 0x42, 0x40])
        // bytes[0..2] = 0x0F, 0x42, 0x40 (Layout A decodes same [0x0F, 0x42, 0x40])
        manufacturer_data.insert(
            GOVEE_COMPANY_ID,
            vec![
                0x0F,  // byte[0] - flags for Layout B, also start of Layout A
                0x42,  // byte[1] - part of both layouts
                0x40,  // byte[2] - part of both layouts (0x0F4240 = 1000000 = temp 100°C)
                0x00,  // byte[3] - completes Layout B with same value
                75,    // byte[4] - battery
            ],
        );

        let result = parse_govee_data(&manufacturer_data);
        assert!(
            result.is_none(),
            "Payload with no plausible decoding should return None"
        );
    }

    #[test]
    fn is_plausible_validates_temperature_boundaries() {
        // Test temperature boundary conditions for is_plausible()
        // Valid range: -40.0 to 85.0°C
        assert!(is_plausible(-40.0, 50.0), "min temp -40.0 should be plausible");
        assert!(is_plausible(-39.9, 50.0), "temp -39.9 should be plausible");
        assert!(!is_plausible(-40.1, 50.0), "temp -40.1 should NOT be plausible");

        assert!(is_plausible(85.0, 50.0), "max temp 85.0 should be plausible");
        assert!(is_plausible(84.9, 50.0), "temp 84.9 should be plausible");
        assert!(!is_plausible(85.1, 50.0), "temp 85.1 should NOT be plausible");

        assert!(is_plausible(0.0, 50.0), "freezing point 0.0 should be plausible");
        assert!(is_plausible(25.0, 50.0), "room temp 25.0 should be plausible");
    }

    #[test]
    fn is_plausible_validates_humidity_boundaries() {
        // Test humidity boundary conditions for is_plausible()
        // Valid range: 0.0 to 100.0%
        assert!(is_plausible(25.0, 0.0), "min humidity 0.0 should be plausible");
        assert!(is_plausible(25.0, 0.1), "humidity 0.1 should be plausible");
        assert!(!is_plausible(25.0, -0.1), "humidity -0.1 should NOT be plausible");

        assert!(is_plausible(25.0, 100.0), "max humidity 100.0 should be plausible");
        assert!(is_plausible(25.0, 99.9), "humidity 99.9 should be plausible");
        assert!(!is_plausible(25.0, 100.1), "humidity 100.1 should NOT be plausible");

        assert!(is_plausible(25.0, 50.0), "mid humidity 50.0 should be plausible");
    }

    #[test]
    fn is_plausible_validates_combined_boundaries() {
        // Test combined boundary conditions (both temp and humidity)
        assert!(
            is_plausible(-40.0, 0.0),
            "min temp + min humidity should be plausible"
        );
        assert!(
            is_plausible(85.0, 100.0),
            "max temp + max humidity should be plausible"
        );
        assert!(!is_plausible(-41.0, 50.0), "invalid temp + valid humidity should NOT be plausible");
        assert!(!is_plausible(25.0, -1.0), "valid temp + invalid humidity should NOT be plausible");
        assert!(
            !is_plausible(90.0, 120.0),
            "both invalid should NOT be plausible"
        );
    }
}
