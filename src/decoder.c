// A thin C surface over libavcodec's HEVC decoder for the browser.
//
// One access unit in, at most one picture out: the decoder runs slice threads
// only, which decode a picture's wavefront rows in parallel and hold nothing back,
// so a unit that completes a picture returns it from the same call. Frame threads
// would hold each picture behind the next ones, and a still desktop sends no next
// one. (A stream with B-frames would still be held back by its reordering; the
// Mac's has none.)
//
// The caller reads the picture's planes straight out of linear memory through
// `hevc_picture`, and releases it with the next call.

#include <stdint.h>
#include <string.h>

#include <emscripten/emscripten.h>

#include "libavcodec/avcodec.h"
#include "libavutil/frame.h"
#include "libavutil/pixdesc.h"

typedef struct Decoder {
    AVCodecContext *ctx;
    AVPacket *packet;
    AVFrame *frame;
    // What `hevc_picture` returns, in the order remotex's hevcWasm.worker.ts reads it.
    int32_t picture[16];
} Decoder;

EMSCRIPTEN_KEEPALIVE
Decoder *hevc_create(int threads)
{
    const AVCodec *codec = avcodec_find_decoder(AV_CODEC_ID_HEVC);
    if (!codec)
        return NULL;
    Decoder *d = av_mallocz(sizeof(*d));
    if (!d)
        return NULL;
    d->ctx = avcodec_alloc_context3(codec);
    d->packet = av_packet_alloc();
    d->frame = av_frame_alloc();
    if (!d->ctx || !d->packet || !d->frame)
        goto fail;
    d->ctx->thread_count = threads;
    d->ctx->thread_type = FF_THREAD_SLICE;
    d->ctx->flags |= AV_CODEC_FLAG_LOW_DELAY;
    d->ctx->flags2 |= AV_CODEC_FLAG2_FAST;
    if (avcodec_open2(d->ctx, codec, NULL) < 0)
        goto fail;
    return d;
fail:
    avcodec_free_context(&d->ctx);
    av_packet_free(&d->packet);
    av_frame_free(&d->frame);
    av_free(d);
    return NULL;
}

// The buffer an access unit is copied into before `hevc_decode`, padded as
// libavcodec's bitstream readers want. Valid until the next call on `d`.
EMSCRIPTEN_KEEPALIVE
uint8_t *hevc_input(Decoder *d, int size)
{
    av_frame_unref(d->frame);
    av_packet_unref(d->packet);
    if (av_new_packet(d->packet, size) < 0)
        return NULL;
    return d->packet->data;
}

// Decode the unit written through `hevc_input`. Returns 1 with a picture for
// `hevc_picture`, 0 when the unit completed none, or a negative AVERROR.
EMSCRIPTEN_KEEPALIVE
int hevc_decode(Decoder *d, int keyframe)
{
    if (keyframe)
        d->packet->flags |= AV_PKT_FLAG_KEY;
    int ret = avcodec_send_packet(d->ctx, d->packet);
    av_packet_unref(d->packet);
    if (ret < 0)
        return ret;
    ret = avcodec_receive_frame(d->ctx, d->frame);
    if (ret == AVERROR(EAGAIN))
        return 0;
    if (ret < 0)
        return ret;
    // Slice threads return every picture from the call that completed it, so a
    // second one here would be a decoder this file does not describe.
    return 1;
}

// The picture `hevc_decode` returned: its size, layout and color, then each
// plane's address and stride.
EMSCRIPTEN_KEEPALIVE
const int32_t *hevc_picture(Decoder *d)
{
    const AVFrame *f = d->frame;
    const AVPixFmtDescriptor *desc = av_pix_fmt_desc_get(f->format);
    int32_t *p = d->picture;
    p[0] = f->width;
    p[1] = f->height;
    // 0 = 4:2:0, 1 = 4:2:2, 2 = 4:4:4, -1 = anything a VideoFrame cannot hold:
    // more than eight bits, or planes that are not Y'CbCr.
    p[2] = -1;
    if (desc && !(desc->flags & (AV_PIX_FMT_FLAG_RGB | AV_PIX_FMT_FLAG_ALPHA)) &&
        desc->nb_components == 3 && desc->comp[0].depth == 8 &&
        (desc->flags & AV_PIX_FMT_FLAG_PLANAR) && desc->log2_chroma_h <= 1) {
        if (desc->log2_chroma_w == 1 && desc->log2_chroma_h == 1)
            p[2] = 0;
        else if (desc->log2_chroma_w == 1 && desc->log2_chroma_h == 0)
            p[2] = 1;
        else if (desc->log2_chroma_w == 0 && desc->log2_chroma_h == 0)
            p[2] = 2;
    }
    p[3] = f->color_range;
    p[4] = f->colorspace;
    p[5] = f->color_primaries;
    p[6] = f->color_trc;
    for (int i = 0; i < 3; i++) {
        p[7 + i] = (int32_t)(uintptr_t)f->data[i];
        p[10 + i] = f->linesize[i];
    }
    p[13] = f->flags & AV_FRAME_FLAG_KEY ? 1 : 0;
    return p;
}

EMSCRIPTEN_KEEPALIVE
void hevc_destroy(Decoder *d)
{
    if (!d)
        return;
    avcodec_free_context(&d->ctx);
    av_packet_free(&d->packet);
    av_frame_free(&d->frame);
    av_free(d);
}
