/*
 * adip_num.c - Scaled integer arithmetic.
 *
 * The whole of libadip's numeric handling. There is no floating point here,
 * and no allocation. Every operation either produces an exact answer or
 * reports that it cannot, because a range check against a safety limit that is
 * approximately right is not right.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip_types.h"

/* 10^n for n in 0..18. int64 holds up to 10^18. */
static const int64_t adip_pow10[ADIP_NUM_MAX_SHIFT + 1] = {
    1LL,
    10LL,
    100LL,
    1000LL,
    10000LL,
    100000LL,
    1000000LL,
    10000000LL,
    100000000LL,
    1000000000LL,
    10000000000LL,
    100000000000LL,
    1000000000000LL,
    10000000000000LL,
    100000000000000LL,
    1000000000000000LL,
    10000000000000000LL,
    100000000000000000LL,
    1000000000000000000LL
};

/*
 * Scale m by 10^shift, reporting overflow rather than wrapping.
 *
 * Signed overflow is undefined behaviour in C, so the check is done before the
 * multiply rather than by inspecting the result afterwards.
 */
static bool adip_scale(int64_t m, int shift, int64_t *out)
{
    int64_t factor;
    int64_t limit;

    if (shift < 0 || shift > ADIP_NUM_MAX_SHIFT) {
        return false;
    }

    factor = adip_pow10[shift];

    if (m != 0) {
        limit = INT64_MAX / factor;
        if (m > limit || m < -limit) {
            return false;
        }
    }

    *out = m * factor;
    return true;
}

bool adip_num_align(adip_num_t a, adip_num_t b, int64_t *out_a, int64_t *out_b)
{
    int target;
    int64_t sa;
    int64_t sb;

    if (out_a == NULL || out_b == NULL) {
        return false;
    }

    /*
     * A unit mismatch is not a comparison that happens to fail. It means the
     * engine sent grams where millilitres were declared, which is a defect.
     * Report it so the caller rejects rather than guessing.
     */
    if (a.unit != b.unit) {
        return false;
    }

    /* Normalise to the smaller exponent, which is the finer scale, so no
     * precision is discarded. */
    target = (a.exponent < b.exponent) ? a.exponent : b.exponent;

    if (!adip_scale((int64_t)a.mantissa, a.exponent - target, &sa)) {
        return false;
    }
    if (!adip_scale((int64_t)b.mantissa, b.exponent - target, &sb)) {
        return false;
    }

    *out_a = sa;
    *out_b = sb;
    return true;
}

adip_cmp_t adip_num_cmp(adip_num_t a, adip_num_t b)
{
    int64_t sa;
    int64_t sb;

    if (!adip_num_align(a, b, &sa, &sb)) {
        return ADIP_CMP_INCOMPARABLE;
    }

    if (sa < sb) {
        return ADIP_CMP_LT;
    }
    if (sa > sb) {
        return ADIP_CMP_GT;
    }
    return ADIP_CMP_EQ;
}

bool adip_num_in_range(adip_num_t v, adip_num_t lo, adip_num_t hi)
{
    adip_cmp_t c;

    c = adip_num_cmp(v, lo);
    if (c == ADIP_CMP_INCOMPARABLE || c == ADIP_CMP_LT) {
        return false;
    }

    c = adip_num_cmp(v, hi);
    if (c == ADIP_CMP_INCOMPARABLE || c == ADIP_CMP_GT) {
        return false;
    }

    return true;
}

bool adip_num_on_resolution(adip_num_t v, adip_num_t lo, adip_num_t resolution)
{
    int64_t sv;
    int64_t slo;
    int64_t sres;
    int64_t sres_lo;
    int64_t delta;

    /* A resolution of zero means the parameter is continuous. */
    if (resolution.mantissa == 0) {
        return true;
    }

    if (!adip_num_align(v, lo, &sv, &slo)) {
        return false;
    }

    /*
     * Align the resolution to the same scale as the offset. Both comparisons
     * go through the same path so a mixed-exponent declaration behaves the
     * same as a uniform one.
     */
    {
        adip_num_t offset;
        offset.mantissa = 0;
        offset.exponent = (v.exponent < lo.exponent) ? v.exponent : lo.exponent;
        offset.unit = v.unit;

        if (!adip_num_align(resolution, offset, &sres, &sres_lo)) {
            return false;
        }
    }

    if (sres == 0) {
        return true;
    }
    if (sres < 0) {
        sres = -sres;
    }

    delta = sv - slo;
    if (delta < 0) {
        delta = -delta;
    }

    return (delta % sres) == 0;
}

#ifdef ADIP_ENABLE_FLOAT
double adip_num_to_double(adip_num_t n)
{
    double v = (double)n.mantissa;
    int e = n.exponent;

    while (e > 0) {
        v *= 10.0;
        e--;
    }
    while (e < 0) {
        v /= 10.0;
        e++;
    }
    return v;
}
#endif

adip_num_t adip_num_neg(adip_num_t a)
{
    adip_num_t r = a;
    /* INT32_MIN has no positive counterpart. Clamping here would be a lie, so
     * it is left alone and callers see an unchanged value, which fails the
     * subsequent range check rather than wrapping to a positive number. */
    if (a.mantissa != INT32_MIN) {
        r.mantissa = -a.mantissa;
    }
    return r;
}

bool adip_num_add(adip_num_t a, adip_num_t b, adip_num_t *out)
{
    int64_t sa;
    int64_t sb;
    int64_t sum;

    if (out == NULL) {
        return false;
    }
    if (!adip_num_align(a, b, &sa, &sb)) {
        return false;
    }

    /* Check before adding. Signed overflow is undefined behaviour, so it
     * cannot be detected after the fact. */
    if (sb > 0 && sa > INT64_MAX - sb) {
        return false;
    }
    if (sb < 0 && sa < INT64_MIN - sb) {
        return false;
    }
    sum = sa + sb;

    if (sum > INT32_MAX || sum < INT32_MIN) {
        return false;
    }

    out->mantissa = (int32_t)sum;
    out->exponent = (a.exponent < b.exponent) ? a.exponent : b.exponent;
    out->unit = a.unit;
    return true;
}
