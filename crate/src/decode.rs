//! The frame decoder. Rice decoding of the residuals, the adaptive FIR
//! predictor that turns them back into samples, and the stereo deinterlacing.

use crate::bits::{shl_i32, shl_i64, shr_i32, shr_i64, shr_u32, Bits};
use crate::Config;

/// The most bits a rice prefix can take before the value is read raw.
const RICE_THRESHOLD: i32 = 8;

/// Sign extends a value of `bits` bits to fill an `i32`.
pub fn sign_extended32(value: i32, bits: i32) -> i32 {
    shr_i32(shl_i32(value, 32 - bits), 32 - bits)
}

/// Sign extends a twenty four bit value to fill an `i32`.
pub fn sign_extend24(value: i32) -> i32 {
    shr_i32(shl_i32(value, 8), 8)
}

/// The sign of a value, as -1, 0 or 1.
fn sign_only(value: i64) -> i64 {
    match value {
        v if v < 0 => -1,
        v if v > 0 => 1,
        _ => 0,
    }
}

/// How many zero bits a value has above its highest one bit.
pub fn count_leading_zeros(input: i64) -> i32 {
    let mut output = 0;

    // the byte the highest one bit is in, found a byte at a time from the top
    let mut curbyte = 'found: {
        let byte = shr_i64(input, 24);
        if byte > 0 {
            break 'found byte;
        }
        output += 8;

        let byte = shr_i64(input, 16);
        if byte & 0xff > 0 {
            break 'found byte;
        }
        output += 8;

        let byte = shr_i64(input, 8);
        if byte & 0xff > 0 {
            break 'found byte;
        }
        output += 8;

        if input & 0xff > 0 {
            break 'found input;
        }
        return output + 8;
    };

    if (curbyte & 0xf0) == 0 {
        output += 4;
    } else {
        curbyte >>= 4;
    }

    if curbyte & 0x8 > 0 {
        return output;
    }
    if curbyte & 0x4 > 0 {
        return output + 1;
    }
    if curbyte & 0x2 > 0 {
        return output + 2;
    }
    if curbyte & 0x1 > 0 {
        return output + 3;
    }

    // not reachable, since one of the four bits above has to be set by then
    output + 4
}

/// Reads one rice coded value.
fn entropy_decode_value(
    bits: &mut Bits,
    read_sample_size: i32,
    k: i32,
    k_modifier_mask: i64,
) -> i32 {
    let mut x: i32 = 0;

    // the count of one bits before the first zero bit is the value
    while x <= RICE_THRESHOLD && bits.read_bit() != 0 {
        x += 1;
    }

    if x > RICE_THRESHOLD {
        // past the threshold the value is written out in full
        let value = bits.read(read_sample_size) as i32;
        x = value & shr_u32(0xffff_ffff, 32 - read_sample_size) as i32;
    } else if k != 1 {
        let extra_bits = bits.read(k) as i64;

        // a negative `k` shifts the one clean away, leaving a scale of -1
        let scale = (shl_i64(1, k) - 1) & k_modifier_mask;
        x = (i64::from(x) * scale) as i32;

        if extra_bits > 1 {
            x = x.wrapping_add((extra_bits - 1) as i32);
        } else {
            bits.unread(1);
        }
    }

    x
}

/// Reads a run of rice coded values into a buffer.
#[allow(clippy::too_many_arguments)]
fn entropy_rice_decode(
    bits: &mut Bits,
    output: &mut [i32],
    output_size: usize,
    read_sample_size: i32,
    rice_initial_history: i64,
    rice_k_modifier: i64,
    rice_history_mult: i64,
    rice_k_modifier_mask: i64,
) {
    let mut history = rice_initial_history;
    let mut sign_modifier: i64 = 0;
    let mut output_count = 0usize;

    while output_count < output_size {
        let mut k = 31 - rice_k_modifier - i64::from(count_leading_zeros(shr_i64(history, 9) + 3));

        if k < 0 {
            k += rice_k_modifier;
        } else {
            k = rice_k_modifier;
        }

        // the mask is deliberately left open here, unlike the run of zeros below
        let mut decoded_value = i64::from(entropy_decode_value(
            bits,
            read_sample_size,
            k as i32,
            0xFFFF_FFFF,
        ));

        decoded_value += sign_modifier;
        let mut final_value = ((decoded_value + 1) / 2) as i32;
        if decoded_value & 1 != 0 {
            // the sign is kept in the lowest bit
            final_value = final_value.wrapping_neg();
        }

        if output_count < output.len() {
            output[output_count] = final_value;
        }

        sign_modifier = 0;

        history += (decoded_value * rice_history_mult) - shr_i64(history * rice_history_mult, 9);

        if decoded_value > 0xFFFF {
            history = 0xFFFF;
        }

        // a run of zeros is written as its length rather than one by one
        if history < 128 && output_count + 1 < output_size {
            sign_modifier = 1;

            k = i64::from(count_leading_zeros(history)) + ((history + 16) / 64) - 24;

            let block_size = i64::from(entropy_decode_value(
                bits,
                16,
                k as i32,
                rice_k_modifier_mask,
            ));

            if block_size > 0 {
                let from = output_count + 1;
                let to = (from + block_size as usize).min(output.len());
                for slot in output.iter_mut().take(to).skip(from) {
                    *slot = 0;
                }
                output_count += block_size as usize;
            }

            if block_size > 0xFFFF {
                sign_modifier = 0;
            }

            history = 0;
        }

        output_count += 1;
    }
}

/// Turns prediction errors back into samples, adapting the filter as it goes.
fn predictor_decompress_fir_adapt(
    error_buffer: &[i32],
    buffer_out: &mut [i32],
    output_size: usize,
    read_sample_size: i32,
    mut predictor_coef_table: [i16; 32],
    predictor_coef_num: usize,
    predictor_quantitization: i32,
) {
    if output_size == 0 || error_buffer.is_empty() || buffer_out.is_empty() {
        return;
    }

    // the first sample is always taken as it stands
    buffer_out[0] = error_buffer[0];

    if predictor_coef_num == 0 {
        if output_size <= 1 {
            return;
        }
        buffer_out[1..output_size].copy_from_slice(&error_buffer[1..output_size]);
        return;
    }

    if predictor_coef_num == 0x1f {
        // the error is a small step away from the sample before it
        if output_size <= 1 {
            return;
        }
        for i in 0..output_size - 1 {
            let previous = buffer_out[i];
            let error = error_buffer[i + 1];
            buffer_out[i + 1] = sign_extended32(previous.wrapping_add(error), read_sample_size);
        }
        return;
    }

    // the samples the filter starts from
    let warm_up = predictor_coef_num
        .min(buffer_out.len() - 1)
        .min(error_buffer.len() - 1);
    for i in 0..warm_up {
        let value = buffer_out[i].wrapping_add(error_buffer[i + 1]);
        buffer_out[i + 1] = sign_extended32(value, read_sample_size);
    }

    // `base` is where the filter's window starts, and it slides forward one
    // sample at a time. the counter reads two buffers at four different
    // offsets, so it stays a counter
    #[allow(clippy::needless_range_loop)]
    for i in predictor_coef_num + 1..output_size {
        let base = i - predictor_coef_num - 1;
        let mut sum: i64 = 0;
        let mut error_val = i64::from(error_buffer[i]);

        for j in 0..predictor_coef_num {
            let difference =
                i64::from(buffer_out[base + predictor_coef_num - j].wrapping_sub(buffer_out[base]));
            sum += (difference * i64::from(predictor_coef_table[j])) as i32 as i64;
        }

        let mut outval = shl_i64(1, predictor_quantitization - 1) + sum;
        outval = shr_i64(outval, predictor_quantitization);
        outval = outval + i64::from(buffer_out[base]) + error_val;
        outval = i64::from(sign_extended32(outval as i32, read_sample_size));

        buffer_out[base + predictor_coef_num + 1] = outval as i32;

        if error_val > 0 {
            let mut predictor_num = predictor_coef_num as i64 - 1;

            while predictor_num >= 0 && error_val > 0 {
                let at = base + predictor_coef_num - predictor_num as usize;
                let mut value = i64::from(buffer_out[base].wrapping_sub(buffer_out[at]));
                let sign = sign_only(value);

                predictor_coef_table[predictor_num as usize] =
                    predictor_coef_table[predictor_num as usize].wrapping_sub(sign as i16);

                value *= sign;

                error_val -= shr_i64(value, predictor_quantitization)
                    * (predictor_coef_num as i64 - predictor_num);

                predictor_num -= 1;
            }
        } else if error_val < 0 {
            let mut predictor_num = predictor_coef_num as i64 - 1;

            while predictor_num >= 0 && error_val < 0 {
                let at = base + predictor_coef_num - predictor_num as usize;
                let mut value = i64::from(buffer_out[base].wrapping_sub(buffer_out[at]));
                let sign = -sign_only(value);

                predictor_coef_table[predictor_num as usize] =
                    predictor_coef_table[predictor_num as usize].wrapping_sub(sign as i16);

                value *= sign;

                error_val -= shr_i64(value, predictor_quantitization)
                    * (predictor_coef_num as i64 - predictor_num);

                predictor_num -= 1;
            }
        }
    }
}

/// Puts two sixteen bit channels back together, little endian.
fn deinterlace_16(
    buffer_a: &[i32],
    buffer_b: &[i32],
    buffer_out: &mut [u8],
    num_channels: usize,
    num_samples: usize,
    interlacing_shift: u8,
    interlacing_leftweight: u8,
) {
    for i in 0..num_samples {
        let (left, right) = if interlacing_leftweight != 0 {
            let midright = buffer_a[i];
            let difference = buffer_b[i];
            let right = midright.wrapping_sub(shr_i32(
                difference.wrapping_mul(i32::from(interlacing_leftweight)),
                i32::from(interlacing_shift),
            )) as i16;
            let left = right.wrapping_add(difference as i16);
            (left, right)
        } else {
            (buffer_a[i] as i16, buffer_b[i] as i16)
        };

        let at = 2 * i * num_channels;
        if at + 3 >= buffer_out.len() {
            return;
        }
        buffer_out[at] = left as u8;
        buffer_out[at + 1] = (left >> 8) as u8;
        buffer_out[at + 2] = right as u8;
        buffer_out[at + 3] = (right >> 8) as u8;
    }
}

/// Puts two twenty four bit channels back together, little endian.
#[allow(clippy::too_many_arguments)]
fn deinterlace_24(
    buffer_a: &[i32],
    buffer_b: &[i32],
    uncompressed_bytes: i32,
    uncompressed_a: &[i32],
    uncompressed_b: &[i32],
    buffer_out: &mut [u8],
    num_channels: usize,
    num_samples: usize,
    interlacing_shift: u8,
    interlacing_leftweight: u8,
) {
    for i in 0..num_samples {
        let (mut left, mut right) = if interlacing_leftweight > 0 {
            let midright = buffer_a[i];
            let difference = buffer_b[i];
            let right = midright.wrapping_sub(shr_i32(
                difference.wrapping_mul(i32::from(interlacing_leftweight)),
                i32::from(interlacing_shift),
            ));
            let left = right.wrapping_add(difference);
            (left, right)
        } else {
            (buffer_a[i], buffer_b[i])
        };

        if uncompressed_bytes > 0 {
            let mask = !shl_i32(-1, uncompressed_bytes * 8);
            left = shl_i32(left, uncompressed_bytes * 8);
            right = shl_i32(right, uncompressed_bytes * 8);
            left |= uncompressed_a[i] & mask;
            right |= uncompressed_b[i] & mask;
        }

        let at = i * num_channels * 3;
        if at + 5 >= buffer_out.len() {
            return;
        }
        buffer_out[at] = (left & 0xFF) as u8;
        buffer_out[at + 1] = ((left >> 8) & 0xFF) as u8;
        buffer_out[at + 2] = ((left >> 16) & 0xFF) as u8;
        buffer_out[at + 3] = (right & 0xFF) as u8;
        buffer_out[at + 4] = ((right >> 8) & 0xFF) as u8;
        buffer_out[at + 5] = ((right >> 16) & 0xFF) as u8;
    }
}

/// The buffers a decoder works in, held between frames so nothing is allocated
/// per frame apart from the answer.
pub struct Buffers {
    pub predict_error_a: Vec<i32>,
    pub predict_error_b: Vec<i32>,
    pub output_samples_a: Vec<i32>,
    pub output_samples_b: Vec<i32>,
    pub uncompressed_a: Vec<i32>,
    pub uncompressed_b: Vec<i32>,
}

impl Buffers {
    /// Buffers for a decoder of the given frame size.
    pub fn new(max_samples_per_frame: u32) -> Self {
        let size = (max_samples_per_frame as usize) * 4;
        Buffers {
            predict_error_a: vec![0; size],
            predict_error_b: vec![0; size],
            output_samples_a: vec![0; size],
            output_samples_b: vec![0; size],
            uncompressed_a: vec![0; size],
            uncompressed_b: vec![0; size],
        }
    }

    /// How many samples one buffer holds.
    pub fn capacity(&self) -> usize {
        self.output_samples_a.len()
    }
}

/// Decodes one frame into little endian samples.
pub fn decode_frame(config: &Config, buffers: &mut Buffers, frame: &[u8]) -> Vec<u8> {
    let mut bits = Bits::new(frame);
    let mut output_samples = config.max_samples_per_frame as usize;
    let bytes_per_sample = (config.sample_size as usize / 8) * config.channels;

    let channels = bits.read(3);
    let mut output_size = output_samples * bytes_per_sample;

    match channels {
        0 => decode_mono(
            config,
            buffers,
            &mut bits,
            &mut output_samples,
            &mut output_size,
            bytes_per_sample,
        ),
        1 => decode_stereo(
            config,
            buffers,
            &mut bits,
            &mut output_samples,
            &mut output_size,
            bytes_per_sample,
        ),
        _ => Vec::new(),
    }
}

/// The part of a frame header both channel counts share.
struct Header {
    uncompressed_bytes: i32,
    is_not_compressed: bool,
}

/// Reads the front of a frame, and the sample count when the frame names one.
fn read_header(
    bits: &mut Bits,
    buffers: &Buffers,
    output_samples: &mut usize,
    output_size: &mut usize,
    bytes_per_sample: usize,
) -> Header {
    // sixteen bits the decoder does not use
    bits.read(4);
    bits.read(12);

    let has_size = bits.read(1) != 0;
    let uncompressed_bytes = bits.read(2) as i32;
    let is_not_compressed = bits.read(1) != 0;

    if has_size {
        // a frame can hold fewer samples than the stream normally does, and
        // the last frame of a track usually does
        let stated = bits.read(32) as usize;
        *output_samples = stated.min(buffers.capacity());
        *output_size = *output_samples * bytes_per_sample;
    }

    Header {
        uncompressed_bytes,
        is_not_compressed,
    }
}

/// One channel.
fn decode_mono(
    config: &Config,
    buffers: &mut Buffers,
    bits: &mut Bits,
    output_samples: &mut usize,
    output_size: &mut usize,
    bytes_per_sample: usize,
) -> Vec<u8> {
    let header = read_header(bits, buffers, output_samples, output_size, bytes_per_sample);
    let mut uncompressed_bytes = header.uncompressed_bytes;
    let samples = *output_samples;
    let read_sample_size = i32::from(config.sample_size) - (uncompressed_bytes * 8);

    if !header.is_not_compressed {
        let mut predictor_coef_table = [0i16; 32];

        // the interlacing shift and weight, which a single channel has no use for
        bits.read(8);
        bits.read(8);

        let prediction_type = bits.read(4);
        let prediction_quantitization = bits.read(4) as i32;
        let rice_modifier = i64::from(bits.read(3));
        let predictor_coef_num = bits.read(5) as usize;

        for slot in predictor_coef_table.iter_mut().take(predictor_coef_num) {
            *slot = bits.read(16) as i16;
        }

        if uncompressed_bytes != 0 {
            for i in 0..samples {
                buffers.uncompressed_a[i] = bits.read(uncompressed_bytes * 8) as i32;
            }
        }

        entropy_rice_decode(
            bits,
            &mut buffers.predict_error_a,
            samples,
            read_sample_size,
            i64::from(config.rice_initial_history),
            i64::from(config.rice_k_modifier),
            rice_modifier * i64::from(config.rice_history_mult) / 4,
            (1i64 << config.rice_k_modifier) - 1,
        );

        if prediction_type == 0 {
            predictor_decompress_fir_adapt(
                &buffers.predict_error_a,
                &mut buffers.output_samples_a,
                samples,
                read_sample_size,
                predictor_coef_table,
                predictor_coef_num,
                prediction_quantitization,
            );
        }
    } else {
        // stored as it stands
        if config.sample_size <= 16 {
            for i in 0..samples {
                let audiobits = bits.read(i32::from(config.sample_size)) as i32;
                buffers.output_samples_a[i] =
                    sign_extended32(audiobits, i32::from(config.sample_size));
            }
        } else {
            for i in 0..samples {
                let mut audiobits = bits.read(16) as i32;
                audiobits = shl_i32(audiobits, i32::from(config.sample_size) - 16);
                audiobits |= bits.read(i32::from(config.sample_size) - 16) as i32;
                buffers.output_samples_a[i] = sign_extend24(audiobits);
            }
        }
        uncompressed_bytes = 0;
    }

    let mut out = vec![0u8; *output_size];
    match config.sample_size {
        16 => {
            for i in 0..samples {
                let sample = buffers.output_samples_a[i] as i16;
                let at = 2 * i * config.channels;
                if at + 1 >= out.len() {
                    break;
                }
                out[at] = sample as u8;
                out[at + 1] = (sample >> 8) as u8;
            }
        }
        24 => {
            for i in 0..samples {
                let mut sample = buffers.output_samples_a[i];
                if uncompressed_bytes != 0 {
                    sample = shl_i32(sample, uncompressed_bytes * 8);
                    let mask = !shl_i32(-1, uncompressed_bytes * 8);
                    sample |= buffers.uncompressed_a[i] & mask;
                }
                let at = i * config.channels * 3;
                if at + 2 >= out.len() {
                    break;
                }
                out[at] = (sample & 0xFF) as u8;
                out[at + 1] = ((sample >> 8) & 0xFF) as u8;
                out[at + 2] = ((sample >> 16) & 0xFF) as u8;
            }
        }
        _ => {}
    }
    out
}

/// Two channels.
fn decode_stereo(
    config: &Config,
    buffers: &mut Buffers,
    bits: &mut Bits,
    output_samples: &mut usize,
    output_size: &mut usize,
    bytes_per_sample: usize,
) -> Vec<u8> {
    let header = read_header(bits, buffers, output_samples, output_size, bytes_per_sample);
    let mut uncompressed_bytes = header.uncompressed_bytes;
    let samples = *output_samples;
    let read_sample_size = i32::from(config.sample_size) - (uncompressed_bytes * 8) + 1;

    let mut interlacing_shift: u8 = 0;
    let mut interlacing_leftweight: u8 = 0;

    if !header.is_not_compressed {
        interlacing_shift = bits.read(8) as u8;
        interlacing_leftweight = bits.read(8) as u8;

        let mut table_a = [0i16; 32];
        let mut table_b = [0i16; 32];

        let prediction_type_a = bits.read(4);
        let prediction_quantitization_a = bits.read(4) as i32;
        let rice_modifier_a = i64::from(bits.read(3));
        let predictor_coef_num_a = bits.read(5) as usize;
        for slot in table_a.iter_mut().take(predictor_coef_num_a) {
            *slot = bits.read(16) as i16;
        }

        let prediction_type_b = bits.read(4);
        let prediction_quantitization_b = bits.read(4) as i32;
        let rice_modifier_b = i64::from(bits.read(3));
        let predictor_coef_num_b = bits.read(5) as usize;
        for slot in table_b.iter_mut().take(predictor_coef_num_b) {
            *slot = bits.read(16) as i16;
        }

        if uncompressed_bytes != 0 {
            for i in 0..samples {
                buffers.uncompressed_a[i] = bits.read(uncompressed_bytes * 8) as i32;
                buffers.uncompressed_b[i] = bits.read(uncompressed_bytes * 8) as i32;
            }
        }

        entropy_rice_decode(
            bits,
            &mut buffers.predict_error_a,
            samples,
            read_sample_size,
            i64::from(config.rice_initial_history),
            i64::from(config.rice_k_modifier),
            rice_modifier_a * i64::from(config.rice_history_mult) / 4,
            (1i64 << config.rice_k_modifier) - 1,
        );

        if prediction_type_a == 0 {
            predictor_decompress_fir_adapt(
                &buffers.predict_error_a,
                &mut buffers.output_samples_a,
                samples,
                read_sample_size,
                table_a,
                predictor_coef_num_a,
                prediction_quantitization_a,
            );
        }

        entropy_rice_decode(
            bits,
            &mut buffers.predict_error_b,
            samples,
            read_sample_size,
            i64::from(config.rice_initial_history),
            i64::from(config.rice_k_modifier),
            rice_modifier_b * i64::from(config.rice_history_mult) / 4,
            (1i64 << config.rice_k_modifier) - 1,
        );

        if prediction_type_b == 0 {
            predictor_decompress_fir_adapt(
                &buffers.predict_error_b,
                &mut buffers.output_samples_b,
                samples,
                read_sample_size,
                table_b,
                predictor_coef_num_b,
                prediction_quantitization_b,
            );
        }
    } else {
        // stored as it stands
        if config.sample_size <= 16 {
            for i in 0..samples {
                let a = bits.read(i32::from(config.sample_size)) as i32;
                let b = bits.read(i32::from(config.sample_size)) as i32;
                buffers.output_samples_a[i] = sign_extended32(a, i32::from(config.sample_size));
                buffers.output_samples_b[i] = sign_extended32(b, i32::from(config.sample_size));
            }
        } else {
            for i in 0..samples {
                let mut a = bits.read(16) as i32;
                a = shl_i32(a, i32::from(config.sample_size) - 16);
                a |= bits.read(i32::from(config.sample_size) - 16) as i32;

                let mut b = bits.read(16) as i32;
                b = shl_i32(b, i32::from(config.sample_size) - 16);
                b |= bits.read(i32::from(config.sample_size) - 16) as i32;

                buffers.output_samples_a[i] = sign_extend24(a);
                buffers.output_samples_b[i] = sign_extend24(b);
            }
        }
        uncompressed_bytes = 0;
    }

    let mut out = vec![0u8; *output_size];
    match config.sample_size {
        16 => deinterlace_16(
            &buffers.output_samples_a,
            &buffers.output_samples_b,
            &mut out,
            config.channels,
            samples,
            interlacing_shift,
            interlacing_leftweight,
        ),
        24 => deinterlace_24(
            &buffers.output_samples_a,
            &buffers.output_samples_b,
            uncompressed_bytes,
            &buffers.uncompressed_a,
            &buffers.uncompressed_b,
            &mut out,
            config.channels,
            samples,
            interlacing_shift,
            interlacing_leftweight,
        ),
        _ => {}
    }
    out
}
