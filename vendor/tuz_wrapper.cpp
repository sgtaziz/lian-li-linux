// C wrapper around tinyuz compression for Rust FFI.
// Provides a simple memory-to-memory compression API.
//
// MIT License — wraps tinyuz (https://github.com/sisong/tinyuz)

#include "tinyuz/compress/tuz_enc.h"
#include <cstring>
#include <cstdlib>


struct MemStreamInput {
    const unsigned char* data;
    size_t size;
};

struct MemStreamOutput {
    unsigned char* buf;
    size_t capacity;
    size_t written;
};

static hpatch_BOOL mem_read(const hpatch_TStreamInput* stream,
                            hpatch_StreamPos_t readFromPos,
                            unsigned char* out_data,
                            unsigned char* out_data_end) {
    auto* ms = (const MemStreamInput*)stream->streamImport;
    size_t len = (size_t)(out_data_end - out_data);
    if (readFromPos + len > ms->size) return hpatch_FALSE;
    memcpy(out_data, ms->data + readFromPos, len);
    return hpatch_TRUE;
}

static hpatch_BOOL mem_write(const hpatch_TStreamOutput* stream,
                             hpatch_StreamPos_t writeToPos,
                             const unsigned char* data,
                             const unsigned char* data_end) {
    auto* ms = (MemStreamOutput*)stream->streamImport;
    size_t len = (size_t)(data_end - data);
    if (writeToPos + len > ms->capacity) return hpatch_FALSE;
    memcpy(ms->buf + writeToPos, data, len);
    size_t end = (size_t)writeToPos + len;
    if (end > ms->written) ms->written = end;
    return hpatch_TRUE;
}


extern "C" {

/// Compress `input` using tinyuz with the given dictionary size.
///
/// Returns the compressed size on success, or 0 on failure.
/// `output` must be at least `tuz_maxCompressedSize(input_len)` bytes.
size_t tuz_compress_mem(const unsigned char* input, size_t input_len,
                        unsigned char* output, size_t output_capacity,
                        size_t dict_size) {
    MemStreamInput  in_mem  = { input, input_len };
    MemStreamOutput out_mem = { output, output_capacity, 0 };

    hpatch_TStreamInput in_stream;
    memset(&in_stream, 0, sizeof(in_stream));
    in_stream.streamImport = (void*)&in_mem;
    in_stream.streamSize   = input_len;
    in_stream.read         = mem_read;

    hpatch_TStreamOutput out_stream;
    memset(&out_stream, 0, sizeof(out_stream));
    out_stream.streamImport    = (void*)&out_mem;
    out_stream.streamSize      = output_capacity;
    out_stream.read_writed     = nullptr;
    out_stream.write           = mem_write;

    tuz_TCompressProps props = tuz_kDefaultCompressProps;
    props.dictSize   = dict_size;
    props.threadNum  = 1;

    hpatch_StreamPos_t result = tuz_compress(&out_stream, &in_stream, &props);
    if (result == 0 || result > output_capacity) return 0;
    return (size_t)result;
}

/// Returns the maximum compressed output size for a given input length.
size_t tuz_max_compressed_size(size_t input_len) {
    return (size_t)tuz_maxCompressedSize(input_len);
}

} // extern "C"
