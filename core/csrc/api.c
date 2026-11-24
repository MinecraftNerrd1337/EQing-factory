/*
 * Library entry point for autoeq-c.
 *
 * Reproduces the body of main() in autoeq.c exactly (same calls, same
 * order), without reading stdin or calling printf. The variant's sources are
 * compiled as is, with -Dmain=<prefix>_cli_main to free up the main
 * symbol. The prefix (EQING_PREFIX) will later allow several variants
 * to be compiled side by side.
 */

#include "lib.h"

#ifndef EQING_PREFIX
#	define EQING_PREFIX orig
#endif

#define CAT_(a, b) a##_##b
#define CAT(a, b) CAT_(a, b)
#define API(name) CAT(CAT(eqing, EQING_PREFIX), name)

/* Smooth is defined (anonymously) in autoeq.c: we only handle the pointer */
struct eqing_smooth;

const struct eqing_smooth *get_ie_smooth(void);
const struct eqing_smooth *get_oe_smooth(void);

f32 preprocess(
	const f32 *restrict f, const f32 *restrict dst,
	const f32 *restrict src, f32 *restrict r, const struct eqing_smooth *smooth, bool demean);

f32 autoeq(
	i32 steps,
	const Type *restrict types,
	f32 *restrict f0,
	f32 *restrict gain,
	f32 *restrict Q,
	f32 *restrict amp,
	const Lim *restrict f0_lim,
	const Lim *restrict gain_lim,
	const Lim *restrict Q_lim,
	i32 N,
	const f32 *restrict f,
	const f32 *restrict r,
	f32 fs);

extern const Type TYPES[];
extern const Lim F0_LIM[], GAIN_LIM[], Q_LIM[];

typedef struct {
	i32 types[MAX_N];
	f32 f0[MAX_N], gain[MAX_N], q[MAX_N];
	/* amp and max: the two values printed on the first line by the binary */
	f32 amp, max;
	/* mean removed from the residual by preprocess (ignored by main) */
	f32 mean;
} EqingAutoeqResult;

i32 API(k)(void) { return K; }
i32 API(max_n)(void) { return MAX_N; }

/* mode: 'i', 'o' or 'x'; returns 0 on success */
i32 API(autoeq_run)(char mode, i32 N, i32 steps,
	const f32 *dst, const f32 *src, EqingAutoeqResult *out)
{
	const struct eqing_smooth *smooth;
	if (mode == 'i') {
		smooth = get_ie_smooth();
	} else if (mode == 'o') {
		smooth = get_oe_smooth();
	} else if (mode == 'x') {
		smooth = NULL;
	} else {
		return -1;
	}

	if (N <= 0 || N > MAX_N)
		return -2;
	if (steps <= 0)
		return -3;

	const f32
		FS = 48000.f,

		F0 = 20.f,
		F1 = 20000.f,

		L0 = logf(F0),
		L1 = logf(F1),
		LR = L1 - L0;

	f32 f[K];
	FOR_K() f[k] = expf(L0 + LR/(f32)(K - 1)*(f32)k);

	f32 gain[MAX_N], f0[MAX_N], Q[MAX_N], amp = 0.;

	f32 r[K];
	f32 mean = preprocess(f, dst, src, r, smooth, true);

	autoeq(steps, TYPES, f0, gain, Q, &amp, F0_LIM, GAIN_LIM, Q_LIM, N, f, r, FS);

	f32 y[K] = {0};
	FOR_N()
		spectrum(TYPES[n], f0[n], gain[n], Q[n], FS, f, y);

	f32 max = -1e9f;
	FOR_K() max = fmaxf(max, y[k]);

	FOR_N() {
		out->types[n] = (i32)TYPES[n];
		out->f0[n] = f0[n];
		out->gain[n] = gain[n];
		out->q[n] = Q[n];
	}
	out->amp = amp;
	out->max = max;
	out->mean = mean;

	return 0;
}

/* dB response of a filter on the K-point grid (for tests) */
void API(spectrum)(i32 type, f32 f0, f32 gain, f32 Q, f32 fs, const f32 *f, f32 *y)
{
	spectrum((Type)type, f0, gain, Q, fs, f, y);
}
