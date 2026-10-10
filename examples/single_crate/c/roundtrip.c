#include "bindings.h"

#include <stdio.h>
#include <string.h>

int main(void) {
  proto_response response = {
      .tag = proto_response_sample,
      .data.sample = {
          .sequence = 42,
          .sample = {
              .timestamp_us = 1000000,
              .gyro = {.x = 1.0f, .y = -2.0f, .z = 3.5f},
              .acc = {.x = 0.0f, .y = 9.81f, .z = -9.81f},
              .temperature_c = {.is_some = true, .value = 24.5f},
          },
      },
  };
  uint8_t buffer[128];
  postcard_slice_t slice = postcard_slice_new(sizeof buffer, buffer);
  if (proto_response_encode(&slice, &response) != POSTCARD_SUCCESS)
    return 1;
  size_t written = slice.len;

  proto_response decoded = {0};
  slice = postcard_slice_new(written, buffer);
  if (proto_response_decode(&slice, &decoded) != POSTCARD_SUCCESS)
    return 2;
  if (slice.len != written || decoded.tag != proto_response_sample ||
      decoded.data.sample.sequence != 42 ||
      decoded.data.sample.sample.gyro.y != -2.0f ||
      decoded.data.sample.sample.acc.z != -9.81f ||
      !decoded.data.sample.sample.temperature_c.is_some ||
      decoded.data.sample.sample.temperature_c.value != 24.5f)
    return 3;

  uint8_t roundtrip[128];
  slice = postcard_slice_new(sizeof roundtrip, roundtrip);
  if (proto_response_encode(&slice, &decoded) != POSTCARD_SUCCESS)
    return 4;
  if (slice.len != written || memcmp(buffer, roundtrip, written) != 0)
    return 5;

  /* Emit postcard bytes so a Rust consumer can decode this same response. */
  return fwrite(buffer, 1, written, stdout) == written ? 0 : 6;
}
