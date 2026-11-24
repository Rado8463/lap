#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <csetjmp>
#include <vector>
#include <algorithm>
#include <atomic>
#include <mutex>
#include <thread>

#include "jpeglib.h"

#ifdef _WIN32
#include <windows.h>
#endif

// ---- Parallel decode of a JPEG that has restart markers ------------------------------------
// A baseline/sequential Huffman JPEG whose restart interval divides the MCU row can be cut at
// restart markers into horizontal strips that are decoded independently on several threads. Each
// strip is wrapped into its own small JPEG (same headers, patched height, renumbered restart
// markers) and carries one extra MCU row above and below as context, so the chroma upsampling is
// identical to a decode of the whole picture. The context rows are discarded.
namespace lapjpar {

struct ErrMgr {
  jpeg_error_mgr pub;
  jmp_buf jb;
  char msg[JMSG_LENGTH_MAX];
};
static void err_exit(j_common_ptr c) {
  auto *e = reinterpret_cast<ErrMgr *>(c->err);
  (*c->err->format_message)(c, e->msg);
  longjmp(e->jb, 1);
}

struct Layout {
  size_t sof_height_pos = 0;   // offset of the 2-byte height in the SOF segment
  size_t header_end = 0;       // first byte of entropy-coded data
  size_t eoi_pos = 0;          // offset of the final EOI marker
  unsigned width = 0, height = 0;
  unsigned mcu_w = 8, mcu_h = 8;
  unsigned restart_interval = 0;
  std::vector<size_t> rst;     // offsets of the 0xFF of every restart marker
};

// Returns false when the file is not suitable (progressive, no restart markers, odd layout ...).
static bool analyse(const unsigned char *d, size_t n, Layout &L) {
  if (n < 4 || d[0] != 0xFF || d[1] != 0xD8) return false;
  size_t p = 2;
  bool have_sof = false;
  unsigned nf = 0, max_h = 1, max_v = 1;
  while (p + 4 <= n) {
    if (d[p] != 0xFF) return false;
    while (p < n && d[p] == 0xFF) ++p;       // fill bytes
    if (p >= n) return false;
    unsigned char m = d[p++];
    if (m == 0xD8 || m == 0x01 || (m >= 0xD0 && m <= 0xD7)) continue;
    if (m == 0xD9) return false;
    if (p + 2 > n) return false;
    size_t len = (static_cast<size_t>(d[p]) << 8) | d[p + 1];
    if (len < 2 || p + len > n) return false;
    if (m == 0xC0 || m == 0xC1) {
      if (have_sof || len < 8) return false;
      have_sof = true;
      if (d[p + 2] != 8) return false;       // 8-bit samples only
      L.sof_height_pos = p + 3;
      L.height = (static_cast<unsigned>(d[p + 3]) << 8) | d[p + 4];
      L.width = (static_cast<unsigned>(d[p + 5]) << 8) | d[p + 6];
      nf = d[p + 7];
      if (nf != 1 && nf != 3) return false;
      if (len != 8 + 3 * static_cast<size_t>(nf)) return false;
      for (unsigned i = 0; i < nf; ++i) {
        unsigned hv = d[p + 8 + 3 * i + 1];
        unsigned h = hv >> 4, v = hv & 15;
        if (h < 1 || h > 4 || v < 1 || v > 4) return false;
        if (h > max_h) max_h = h;
        if (v > max_v) max_v = v;
      }
    } else if ((m >= 0xC2 && m <= 0xCF && m != 0xC4 && m != 0xC8 && m != 0xCC)) {
      return false;                          // progressive, lossless, arithmetic ...
    } else if (m == 0xDD) {
      if (len != 4) return false;
      L.restart_interval = (static_cast<unsigned>(d[p + 2]) << 8) | d[p + 3];
    } else if (m == 0xDC) {
      return false;                          // DNL
    } else if (m == 0xDA) {
      if (!have_sof || L.height == 0 || L.width == 0) return false;
      unsigned ns = d[p + 2];
      if (ns != nf) return false;            // one scan with all components
      L.header_end = p + len;
      break;
    }
    p += len;
  }
  if (L.header_end == 0 || L.restart_interval == 0) return false;
  if (nf == 1) { L.mcu_w = 8; L.mcu_h = 8; } else { L.mcu_w = 8 * max_h; L.mcu_h = 8 * max_v; }
  unsigned mcus_per_row = (L.width + L.mcu_w - 1) / L.mcu_w;
  if (mcus_per_row % L.restart_interval != 0) return false;
  // Entropy-coded data: collect restart markers up to the EOI.
  size_t q = L.header_end;
  while (q + 1 < n) {
    if (d[q] != 0xFF) { ++q; continue; }
    unsigned char m = d[q + 1];
    if (m == 0x00) { q += 2; continue; }
    if (m == 0xFF) { ++q; continue; }
    if (m >= 0xD0 && m <= 0xD7) { L.rst.push_back(q); q += 2; continue; }
    if (m == 0xD9) { L.eoi_pos = q; break; }
    return false;                            // another marker inside the scan
  }
  if (L.eoi_pos == 0) return false;
  unsigned mcu_rows = (L.height + L.mcu_h - 1) / L.mcu_h;
  size_t intervals = static_cast<size_t>(mcu_rows) * (mcus_per_row / L.restart_interval);
  return L.rst.size() + 1 == intervals;
}

static bool decode_strip(const unsigned char *d, const Layout &L, unsigned row0, unsigned row1,
                         unsigned ctx0, unsigned ctx1, unsigned char *out, char *err, size_t err_len) {
  // Rows [ctx0, ctx1) are decoded, rows [row0, row1) are kept (MCU rows).
  unsigned mcus_per_row = (L.width + L.mcu_w - 1) / L.mcu_w;
  size_t ipr = mcus_per_row / L.restart_interval;
  size_t i0 = static_cast<size_t>(ctx0) * ipr, i1 = static_cast<size_t>(ctx1) * ipr;
  size_t total = L.rst.size() + 1;
  size_t begin = i0 == 0 ? L.header_end : L.rst[i0 - 1] + 2;
  size_t end = i1 >= total ? L.eoi_pos : L.rst[i1 - 1];
  if (end < begin) return false;
  unsigned strip_px = std::min((ctx1 - ctx0) * L.mcu_h, L.height - ctx0 * L.mcu_h);
  std::vector<unsigned char> buf;
  buf.reserve(L.header_end + (end - begin) + 2);
  buf.insert(buf.end(), d, d + L.header_end);
  buf[L.sof_height_pos] = static_cast<unsigned char>(strip_px >> 8);
  buf[L.sof_height_pos + 1] = static_cast<unsigned char>(strip_px & 255);
  size_t k = 0, r = i0;                      // r indexes L.rst: marker r follows interval r
  size_t cur = begin;
  while (r < i1 - 1 && r < L.rst.size()) {
    size_t mp = L.rst[r];
    buf.insert(buf.end(), d + cur, d + mp);
    buf.push_back(0xFF);
    buf.push_back(static_cast<unsigned char>(0xD0 + (k & 7)));
    cur = mp + 2;
    ++k; ++r;
  }
  buf.insert(buf.end(), d + cur, d + end);
  buf.push_back(0xFF);
  buf.push_back(0xD9);

  jpeg_decompress_struct cinfo;
  std::memset(&cinfo, 0, sizeof(cinfo));
  ErrMgr jerr;
  cinfo.err = jpeg_std_error(&jerr.pub);
  jerr.pub.error_exit = err_exit;
  jerr.msg[0] = 0;
  std::vector<unsigned char> row_buf;
  if (setjmp(jerr.jb)) {
    jpeg_destroy_decompress(&cinfo);
    if (err && err_len) std::snprintf(err, err_len, "%s", jerr.msg);
    return false;
  }
  jpeg_create_decompress(&cinfo);
  jpeg_mem_src(&cinfo, buf.data(), static_cast<unsigned long>(buf.size()));
  jpeg_read_header(&cinfo, TRUE);
  cinfo.out_color_space = JCS_RGB;
  jpeg_start_decompress(&cinfo);
  if (cinfo.output_width != L.width || cinfo.output_height != strip_px) {
    jpeg_destroy_decompress(&cinfo);
    if (err && err_len) std::snprintf(err, err_len, "Unexpected strip size");
    return false;
  }
  const size_t stride = static_cast<size_t>(L.width) * 3;
  row_buf.resize(stride);
  const unsigned skip_rows = (row0 - ctx0) * L.mcu_h;
  const unsigned keep_end = std::min(row1 * L.mcu_h, L.height) - ctx0 * L.mcu_h;
  for (unsigned y = 0; y < keep_end; ++y) {
    unsigned char *rows[1];
    rows[0] = y < skip_rows ? row_buf.data() : out + static_cast<size_t>(y - skip_rows) * stride;
    jpeg_read_scanlines(&cinfo, rows, 1);
  }
  jpeg_destroy_decompress(&cinfo);
  return true;
}

}  // namespace lapjpar

extern "C" {

struct LapJpegErrorManager {
  jpeg_error_mgr pub;
  jmp_buf setjmp_buffer;
  char message[JMSG_LENGTH_MAX];
};

static void lap_jpeg_error_exit(j_common_ptr cinfo) {
  auto *err = reinterpret_cast<LapJpegErrorManager *>(cinfo->err);
  (*cinfo->err->format_message)(cinfo, err->message);
  longjmp(err->setjmp_buffer, 1);
}

static FILE *lap_fopen_utf8(const char *file_path, const char *mode) {
#ifdef _WIN32
  int path_len = MultiByteToWideChar(CP_UTF8, 0, file_path, -1, nullptr, 0);
  int mode_len = MultiByteToWideChar(CP_UTF8, 0, mode, -1, nullptr, 0);
  if (path_len <= 0 || mode_len <= 0) {
    return nullptr;
  }

  std::vector<wchar_t> wide_path(static_cast<size_t>(path_len));
  std::vector<wchar_t> wide_mode(static_cast<size_t>(mode_len));
  if (MultiByteToWideChar(CP_UTF8, 0, file_path, -1, wide_path.data(),
                          path_len) <= 0 ||
      MultiByteToWideChar(CP_UTF8, 0, mode, -1, wide_mode.data(), mode_len) <=
          0) {
    return nullptr;
  }

  return _wfopen(wide_path.data(), wide_mode.data());
#else
  return std::fopen(file_path, mode);
#endif
}

static bool lap_read_file(const char *file_path,
                          std::vector<unsigned char> &out) {
  FILE *file = lap_fopen_utf8(file_path, "rb");
  if (!file) {
    return false;
  }

  unsigned char buffer[64 * 1024];
  while (true) {
    size_t bytes_read = std::fread(buffer, 1, sizeof(buffer), file);
    if (bytes_read > 0) {
      out.insert(out.end(), buffer, buffer + bytes_read);
    }
    if (bytes_read < sizeof(buffer)) {
      bool ok = std::feof(file) != 0;
      std::fclose(file);
      return ok;
    }
  }
}

int lap_jpeg_encode_rgb8_ex(const unsigned char *rgb_data, unsigned int width,
                            unsigned int height, int quality, int full_chroma,
                            unsigned char **out_data, unsigned long *out_len,
                            char *err_buf, size_t err_buf_len) {
  if (!rgb_data || width == 0 || height == 0 || !out_data || !out_len) {
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "Invalid JPEG encode arguments");
    }
    return 0;
  }

  *out_data = nullptr;
  *out_len = 0;

  jpeg_compress_struct cinfo;
  std::memset(&cinfo, 0, sizeof(cinfo));
  LapJpegErrorManager jerr;
  cinfo.err = jpeg_std_error(&jerr.pub);
  jerr.pub.error_exit = lap_jpeg_error_exit;
  jerr.message[0] = '\0';

  if (setjmp(jerr.setjmp_buffer)) {
    jpeg_destroy_compress(&cinfo);
    if (err_buf && err_buf_len > 0) {
      if (jerr.message[0] != '\0') {
        std::snprintf(err_buf, err_buf_len, "%s", jerr.message);
      } else {
        std::snprintf(err_buf, err_buf_len, "libjpeg-turbo encode failed");
      }
    }
    if (*out_data) {
      std::free(*out_data);
      *out_data = nullptr;
    }
    *out_len = 0;
    return 0;
  }

  jpeg_create_compress(&cinfo);
  jpeg_mem_dest(&cinfo, out_data, out_len);
  cinfo.image_width = width;
  cinfo.image_height = height;
  cinfo.input_components = 3;
  cinfo.in_color_space = JCS_RGB;

  jpeg_set_defaults(&cinfo);
  jpeg_set_quality(&cinfo, quality, TRUE);
  if (full_chroma) {
    // 4:4:4 (no chroma subsampling): keeps sharp colour edges of lossless graphics.
    cinfo.comp_info[0].h_samp_factor = 1;
    cinfo.comp_info[0].v_samp_factor = 1;
  }
  jpeg_start_compress(&cinfo, TRUE);

  while (cinfo.next_scanline < cinfo.image_height) {
    JSAMPROW row = const_cast<JSAMPROW>(
        rgb_data + static_cast<size_t>(cinfo.next_scanline) * width * 3);
    jpeg_write_scanlines(&cinfo, &row, 1);
  }

  jpeg_finish_compress(&cinfo);
  jpeg_destroy_compress(&cinfo);
  return 1;
}

int lap_jpeg_encode_rgb8(const unsigned char *rgb_data, unsigned int width,
                         unsigned int height, int quality,
                         unsigned char **out_data, unsigned long *out_len,
                         char *err_buf, size_t err_buf_len) {
  return lap_jpeg_encode_rgb8_ex(rgb_data, width, height, quality, 0, out_data,
                                 out_len, err_buf, err_buf_len);
}

int lap_jpeg_decode_rgb8(const char *file_path, unsigned int target_width,
                         unsigned int target_height, unsigned int *out_width,
                         unsigned int *out_height, unsigned char **out_data,
                         char *err_buf, size_t err_buf_len) {
  if (!file_path || !out_width || !out_height || !out_data) {
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "Invalid JPEG decode arguments");
    }
    return 0;
  }

  // Initialize early to prevent leak in setjmp handler
  *out_data = nullptr;

  std::vector<unsigned char> file_data;
  if (!lap_read_file(file_path, file_data)) {
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "Could not open file: %s", file_path);
    }
    return 0;
  }

  if (file_data.empty()) {
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "File is empty: %s", file_path);
    }
    return 0;
  }

  jpeg_decompress_struct cinfo;
  std::memset(&cinfo, 0, sizeof(cinfo));
  LapJpegErrorManager jerr;
  cinfo.err = jpeg_std_error(&jerr.pub);
  jerr.pub.error_exit = lap_jpeg_error_exit;
  jerr.message[0] = '\0';

  if (setjmp(jerr.setjmp_buffer)) {
    jpeg_destroy_decompress(&cinfo);
    if (*out_data) {
      std::free(*out_data);
      *out_data = nullptr;
    }
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "%s", jerr.message);
    }
    return 0;
  }

  jpeg_create_decompress(&cinfo);
  jpeg_mem_src(&cinfo, file_data.data(),
               static_cast<unsigned long>(file_data.size()));
  jpeg_read_header(&cinfo, TRUE);

  // Calculate scaling factor (libjpeg supports 1/1, 1/2, 1/4, 1/8)
  unsigned int scale_denom = 1;
  if (target_width > 0 && target_height > 0) {
    while (scale_denom < 8 &&
           cinfo.image_width >= target_width * scale_denom * 2 &&
           cinfo.image_height >= target_height * scale_denom * 2) {
      scale_denom *= 2;
    }
  }
  cinfo.scale_num = 1;
  cinfo.scale_denom = scale_denom;
  cinfo.out_color_space = JCS_RGB;

  jpeg_start_decompress(&cinfo);

  *out_width = cinfo.output_width;
  *out_height = cinfo.output_height;
  size_t row_stride = static_cast<size_t>(cinfo.output_width) * 3;
  size_t total_size = row_stride * cinfo.output_height;

  *out_data = static_cast<unsigned char *>(std::malloc(total_size));
  if (!(*out_data)) {
    jpeg_destroy_decompress(&cinfo);
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "Memory allocation failed for JPEG decode");
    }
    return 0;
  }

  while (cinfo.output_scanline < cinfo.output_height) {
    unsigned char *buffer_array[1];
    buffer_array[0] = *out_data + (cinfo.output_scanline * row_stride);
    jpeg_read_scanlines(&cinfo, buffer_array, 1);
  }

  jpeg_finish_decompress(&cinfo);
  jpeg_destroy_decompress(&cinfo);
  return 1;
}

// Decodes only a rectangle of a JPEG. (x, y, w, h) is in source pixels; scale_denom (1, 2, 4, 8)
// selects the DCT downscale. The result is the rectangle at 1/scale_denom size:
// out_width = ceil((x + w) / d) - floor(x / d), likewise for the height. Rows above the
// rectangle are only entropy-decoded (jpeg_skip_scanlines), so it is far cheaper than a full decode.
int lap_jpeg_decode_region_rgb8(const char *file_path, unsigned int x,
                                unsigned int y, unsigned int w, unsigned int h,
                                unsigned int scale_denom,
                                unsigned int *out_width,
                                unsigned int *out_height,
                                unsigned char **out_data, char *err_buf,
                                size_t err_buf_len) {
  if (!file_path || !out_width || !out_height || !out_data || w == 0 ||
      h == 0) {
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "Invalid JPEG region arguments");
    }
    return 0;
  }
  *out_data = nullptr;
  if (scale_denom != 1 && scale_denom != 2 && scale_denom != 4 &&
      scale_denom != 8) {
    scale_denom = 1;
  }

  std::vector<unsigned char> file_data;
  if (!lap_read_file(file_path, file_data) || file_data.empty()) {
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "Could not read file: %s", file_path);
    }
    return 0;
  }

  jpeg_decompress_struct cinfo;
  std::memset(&cinfo, 0, sizeof(cinfo));
  LapJpegErrorManager jerr;
  cinfo.err = jpeg_std_error(&jerr.pub);
  jerr.pub.error_exit = lap_jpeg_error_exit;
  jerr.message[0] = '\0';

  unsigned char *volatile row_buf = nullptr;  // volatile: modified after setjmp
  if (setjmp(jerr.setjmp_buffer)) {
    jpeg_destroy_decompress(&cinfo);
    if (row_buf) {
      std::free(row_buf);
    }
    if (*out_data) {
      std::free(*out_data);
      *out_data = nullptr;
    }
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "%s", jerr.message);
    }
    return 0;
  }

  jpeg_create_decompress(&cinfo);
  jpeg_mem_src(&cinfo, file_data.data(),
               static_cast<unsigned long>(file_data.size()));
  jpeg_read_header(&cinfo, TRUE);
  cinfo.scale_num = 1;
  cinfo.scale_denom = scale_denom;
  cinfo.out_color_space = JCS_RGB;
  cinfo.do_fancy_upsampling = TRUE;  // required by jpeg_crop_scanline
  jpeg_start_decompress(&cinfo);

  const unsigned int full_w = cinfo.output_width;
  const unsigned int full_h = cinfo.output_height;
  unsigned int sx0 = x / scale_denom;
  unsigned int sy0 = y / scale_denom;
  unsigned int sx1 = (x + w + scale_denom - 1) / scale_denom;
  unsigned int sy1 = (y + h + scale_denom - 1) / scale_denom;
  if (sx1 > full_w) sx1 = full_w;
  if (sy1 > full_h) sy1 = full_h;
  if (sx0 >= sx1 || sy0 >= sy1) {
    jpeg_destroy_decompress(&cinfo);
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "JPEG region outside the image");
    }
    return 0;
  }

  // The decoder widens the crop to whole iMCUs (xoff moves left); the requested part is copied out below.
  JDIMENSION xoff = sx0;
  JDIMENSION crop_w = sx1 - sx0;
  jpeg_crop_scanline(&cinfo, &xoff, &crop_w);
  if (sy0 > 0) {
    jpeg_skip_scanlines(&cinfo, sy0);
  }

  const size_t out_w = sx1 - sx0;
  const size_t out_h = sy1 - sy0;
  const size_t row_stride = static_cast<size_t>(cinfo.output_width) * 3;
  row_buf = static_cast<unsigned char *>(std::malloc(row_stride));
  *out_data = static_cast<unsigned char *>(std::malloc(out_w * out_h * 3));
  if (!row_buf || !(*out_data)) {
    if (row_buf) std::free(row_buf);
    if (*out_data) {
      std::free(*out_data);
      *out_data = nullptr;
    }
    jpeg_destroy_decompress(&cinfo);
    if (err_buf && err_buf_len > 0) {
      std::snprintf(err_buf, err_buf_len, "Memory allocation failed for JPEG region");
    }
    return 0;
  }

  const size_t skip_bytes = static_cast<size_t>(sx0 - xoff) * 3;
  for (size_t row = 0; row < out_h; ++row) {
    unsigned char *buffer_array[1];
    buffer_array[0] = row_buf;
    jpeg_read_scanlines(&cinfo, buffer_array, 1);
    std::memcpy(*out_data + row * out_w * 3, row_buf + skip_bytes, out_w * 3);
  }

  std::free(row_buf);
  row_buf = nullptr;
  *out_width = static_cast<unsigned int>(out_w);
  *out_height = static_cast<unsigned int>(out_h);
  // Not all scanlines were read on purpose, so the decoder is destroyed without jpeg_finish_decompress.
  jpeg_destroy_decompress(&cinfo);
  return 1;
}

// Decodes a whole JPEG into `out` (width * height * 3 RGB bytes) on several threads. Returns 1 on
// success, 2 when the file cannot be split (no restart markers, progressive, size mismatch ...; the
// caller decodes it the usual way) and 0 on an error.
int lap_jpeg_decode_parallel_rgb8(const char *file_path, unsigned int width,
                                  unsigned int height, unsigned int nthreads,
                                  unsigned char *out, size_t out_cap,
                                  char *err_buf, size_t err_buf_len) {
  if (!file_path || !out || width == 0 || height == 0) return 0;
  std::vector<unsigned char> file;
  if (!lap_read_file(file_path, file) || file.empty()) {
    if (err_buf && err_buf_len > 0)
      std::snprintf(err_buf, err_buf_len, "Could not read file: %s", file_path);
    return 0;
  }
  lapjpar::Layout L;
  if (!lapjpar::analyse(file.data(), file.size(), L)) return 2;
  if (L.width != width || L.height != height ||
      out_cap < static_cast<size_t>(width) * height * 3)
    return 2;
  const unsigned mcu_rows = (L.height + L.mcu_h - 1) / L.mcu_h;
  const unsigned nt = std::max(1u, nthreads);
  unsigned strips = std::max(1u, std::min(nt * 2, mcu_rows / 8));
  const unsigned per = (mcu_rows + strips - 1) / strips;
  strips = (mcu_rows + per - 1) / per;
  std::atomic<unsigned> next(0);
  std::atomic<bool> failed(false);
  std::mutex mu;
  char msg[512];
  msg[0] = 0;
  auto worker = [&]() {
    for (;;) {
      const unsigned s = next.fetch_add(1);
      if (s >= strips || failed.load()) return;
      const unsigned r0 = s * per, r1 = std::min(mcu_rows, r0 + per);
      const unsigned c0 = r0 > 0 ? r0 - 1 : 0, c1 = std::min(mcu_rows, r1 + 1);
      char e[512];
      e[0] = 0;
      unsigned char *dst = out + static_cast<size_t>(r0) * L.mcu_h * width * 3;
      if (!lapjpar::decode_strip(file.data(), L, r0, r1, c0, c1, dst, e, sizeof(e))) {
        std::lock_guard<std::mutex> g(mu);
        if (!failed.exchange(true)) std::snprintf(msg, sizeof(msg), "%s", e);
        return;
      }
    }
  };
  std::vector<std::thread> pool;
  for (unsigned i = 1; i < std::min(nt, strips); ++i) pool.emplace_back(worker);
  worker();
  for (auto &t : pool) t.join();
  if (failed.load()) {
    if (err_buf && err_buf_len > 0)
      std::snprintf(err_buf, err_buf_len, "%s", msg[0] ? msg : "strip decode failed");
    return 0;
  }
  return 1;
}

void lap_jpeg_free_buffer(unsigned char *data) {
  if (data) {
    std::free(data);
  }
}

}
