/*
 * Copyright (c) 2024, Mozilla Foundation
 *
 * Redistribution and use in source and binary forms, with or without
 * modification, are permitted provided that the following conditions are met:
 *
 * 1. Redistributions of source code must retain the above copyright notice, this
 *    list of conditions and the following disclaimer.
 *
 * 2. Redistributions in binary form must reproduce the above copyright notice,
 *    this list of conditions and the following disclaimer in the documentation
 *    and/or other materials provided with the distribution.
 *
 * THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
 * AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 * IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
 * DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
 * FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
 * DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
 * SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
 * CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
 * OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 * OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 */

mergeInto(LibraryManager.library, {
  jsPrintWarning: function (message_ptr) {
    const message = UTF8ToString(message_ptr);
    (Module.warn || console.warn)(`OpenJPEG: ${message}`);
  },
  setImageData: function (array_ptr, array_size, offset) {
    Module.imageData.set(
      Module.HEAPU8.subarray(array_ptr, array_ptr + array_size),
      offset
    );
  },
  copy_pixels_1(compG_ptr, nb_pixels) {
    compG_ptr >>= 2;
    const imageData = (Module.imageData = new Uint8ClampedArray(nb_pixels));
    const compG = HEAP32.subarray(compG_ptr, compG_ptr + nb_pixels);
    imageData.set(compG);
  },
  copy_pixels_3(compR_ptr, compG_ptr, compB_ptr, nb_pixels) {
    compR_ptr >>= 2;
    compG_ptr >>= 2;
    compB_ptr >>= 2;
    const imageData = (Module.imageData = new Uint8ClampedArray(nb_pixels * 3));
    const compR = HEAP32.subarray(compR_ptr, compR_ptr + nb_pixels);
    const compG = HEAP32.subarray(compG_ptr, compG_ptr + nb_pixels);
    const compB = HEAP32.subarray(compB_ptr, compB_ptr + nb_pixels);
    for (let i = 0; i < nb_pixels; i++) {
      imageData[3 * i] = compR[i];
      imageData[3 * i + 1] = compG[i];
      imageData[3 * i + 2] = compB[i];
    }
  },
  copy_pixels_4(compR_ptr, compG_ptr, compB_ptr, compA_ptr, nb_pixels) {
    compR_ptr >>= 2;
    compG_ptr >>= 2;
    compB_ptr >>= 2;
    compA_ptr >>= 2;
    const imageData = (Module.imageData = new Uint8ClampedArray(nb_pixels * 4));
    const compR = HEAP32.subarray(compR_ptr, compR_ptr + nb_pixels);
    const compG = HEAP32.subarray(compG_ptr, compG_ptr + nb_pixels);
    const compB = HEAP32.subarray(compB_ptr, compB_ptr + nb_pixels);
    const compA = HEAP32.subarray(compA_ptr, compA_ptr + nb_pixels);
    for (let i = 0; i < nb_pixels; i++) {
      imageData[4 * i] = compR[i];
      imageData[4 * i + 1] = compG[i];
      imageData[4 * i + 2] = compB[i];
      imageData[4 * i + 3] = compA[i];
    }
  },
  gray_to_rgba(compG_ptr, nb_pixels) {
    compG_ptr >>= 2;
    const imageData = (Module.imageData = new Uint8ClampedArray(nb_pixels * 4));
    const compG = HEAP32.subarray(compG_ptr, compG_ptr + nb_pixels);
    for (let i = 0; i < nb_pixels; i++) {
      imageData[4 * i] = imageData[4 * i + 1] = imageData[4 * i + 2] = compG[i];
      imageData[4 * i + 3] = 0xff;
    }
  },
  graya_to_rgba(compG_ptr, compA_ptr, nb_pixels) {
    compG_ptr >>= 2;
    compA_ptr >>= 2;
    const imageData = (Module.imageData = new Uint8ClampedArray(nb_pixels * 4));
    const compG = HEAP32.subarray(compG_ptr, compG_ptr + nb_pixels);
    const compA = HEAP32.subarray(compA_ptr, compA_ptr + nb_pixels);
    for (let i = 0; i < nb_pixels; i++) {
      imageData[4 * i] = imageData[4 * i + 1] = imageData[4 * i + 2] = compG[i];
      imageData[4 * i + 3] = compA[i];
    }
  },
  rgb_to_rgba(compR_ptr, compG_ptr, compB_ptr, nb_pixels) {
    compR_ptr >>= 2;
    compG_ptr >>= 2;
    compB_ptr >>= 2;
    const imageData = (Module.imageData = new Uint8ClampedArray(nb_pixels * 4));
    const compR = HEAP32.subarray(compR_ptr, compR_ptr + nb_pixels);
    const compG = HEAP32.subarray(compG_ptr, compG_ptr + nb_pixels);
    const compB = HEAP32.subarray(compB_ptr, compB_ptr + nb_pixels);
    for (let i = 0; i < nb_pixels; i++) {
      imageData[4 * i] = compR[i];
      imageData[4 * i + 1] = compG[i];
      imageData[4 * i + 2] = compB[i];
      imageData[4 * i + 3] = 0xff;
    }
  },
  storeErrorMessage: function (message_ptr) {
    const message = UTF8ToString(message_ptr);
    if (!Module.errorMessages) {
      Module.errorMessages = message;
    } else {
      Module.errorMessages += "\n" + message;
    }
  },
});
