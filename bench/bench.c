// Decode an Annex B file through decoder.c, as the page does, and report the time
// each access unit took and each picture's MD5 — the same digest `ffmpeg -f
// framemd5` prints for rawvideo, so a run can be checked against native FFmpeg.
//
//   node bench.js FILE THREADS [REPEATS]

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <emscripten/emscripten.h>

#include "libavcodec/avcodec.h"
#include "libavutil/md5.h"
#include "libavutil/time.h"

typedef struct Decoder Decoder;
Decoder *hevc_create(int threads);
uint8_t *hevc_input(Decoder *d, int size);
int hevc_decode(Decoder *d, int keyframe);
const int32_t *hevc_picture(Decoder *d);
void hevc_destroy(Decoder *d);

static int cmp(const void *a, const void *b)
{
    double x = *(const double *)a, y = *(const double *)b;
    return x < y ? -1 : x > y;
}

int main(int argc, char **argv)
{
    if (argc < 3) {
        fprintf(stderr, "usage: bench FILE THREADS [REPEATS]\n");
        return 2;
    }
    int threads = atoi(argv[2]);
    int repeats = argc > 3 ? atoi(argv[3]) : 1;
    FILE *f = fopen(argv[1], "rb");
    if (!f) {
        perror(argv[1]);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *file = malloc(size);
    if (fread(file, 1, size, f) != (size_t)size)
        return 1;
    fclose(f);

    // Split the file into access units once, outside the timing.
    AVCodecParserContext *parser = av_parser_init(AV_CODEC_ID_HEVC);
    AVCodecContext *pctx = avcodec_alloc_context3(NULL);
    int cap = 1024, count = 0;
    uint8_t **units = malloc(cap * sizeof(*units));
    int *sizes = malloc(cap * sizeof(*sizes));
    int *keys = malloc(cap * sizeof(*keys));
    const uint8_t *p = file;
    long left = size;
    while (1) {
        uint8_t *out;
        int out_size;
        int used = av_parser_parse2(parser, pctx, &out, &out_size, p, (int)left,
                                    AV_NOPTS_VALUE, AV_NOPTS_VALUE, 0);
        p += used;
        left -= used;
        if (out_size) {
            if (count == cap) {
                cap *= 2;
                units = realloc(units, cap * sizeof(*units));
                sizes = realloc(sizes, cap * sizeof(*sizes));
                keys = realloc(keys, cap * sizeof(*keys));
            }
            units[count] = malloc(out_size);
            memcpy(units[count], out, out_size);
            sizes[count] = out_size;
            keys[count] = parser->key_frame == 1;
            count++;
        }
        // An empty call flushes the unit the parser still holds.
        if (!left && !out_size && !used && p == file + size)
            break;
    }

    double *ms = malloc(count * repeats * sizeof(*ms));
    int pictures = 0, n = 0;
    int64_t start = av_gettime_relative();
    for (int r = 0; r < repeats; r++) {
        Decoder *d = hevc_create(threads);
        if (!d) {
            fprintf(stderr, "hevc_create failed\n");
            return 1;
        }
        for (int i = 0; i < count; i++) {
            int64_t t = av_gettime_relative();
            memcpy(hevc_input(d, sizes[i]), units[i], sizes[i]);
            int ret = hevc_decode(d, keys[i]);
            ms[n++] = (av_gettime_relative() - t) / 1000.0;
            if (ret < 0) {
                fprintf(stderr, "unit %d: %s\n", i, av_err2str(ret));
                return 1;
            }
            if (ret == 0)
                continue;
            pictures++;
            if (r > 0)
                continue;
            const int32_t *pic = hevc_picture(d);
            int w = pic[0], h = pic[1], shift = pic[2] == 2 ? 0 : 1;
            uint8_t digest[16];
            struct AVMD5 *md5 = av_md5_alloc();
            av_md5_init(md5);
            for (int plane = 0; plane < 3; plane++) {
                int pw = plane ? (w + shift) >> shift : w;
                int ph = plane ? (pic[2] == 0 ? (h + 1) >> 1 : h) : h;
                const uint8_t *row = (const uint8_t *)(uintptr_t)pic[7 + plane];
                for (int y = 0; y < ph; y++)
                    av_md5_update(md5, row + (ptrdiff_t)y * pic[10 + plane], pw);
            }
            av_md5_final(md5, digest);
            av_free(md5);
            printf("md5 %d ", i);
            for (int k = 0; k < 16; k++)
                printf("%02x", digest[k]);
            printf("\n");
        }
        hevc_destroy(d);
    }
    double total = (av_gettime_relative() - start) / 1000.0;
    qsort(ms, n, sizeof(*ms), cmp);
    double sum = 0;
    for (int i = 0; i < n; i++)
        sum += ms[i];
    fprintf(stderr,
            "threads %d: %d units, %d pictures, %.0f ms; per unit mean %.2f "
            "p50 %.2f p95 %.2f max %.2f ms; %.1f fps\n",
            threads, n, pictures, total, sum / n, ms[n / 2], ms[n * 95 / 100],
            ms[n - 1], pictures * 1000.0 / total);
    return 0;
}
