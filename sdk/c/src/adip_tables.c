/*
 * adip_tables.c - Lookups over the generated tables.
 *
 * Linear scans on purpose. A declaration has a handful of tools, not
 * thousands, and a scan over a const array in flash beats any structure that
 * would need building at startup. It is also trivially bounded, which matters
 * more here than speed.
 *
 * Copyright 2026 StreetAI. Licensed under the Apache License, Version 2.0.
 */

#include "adip_tables.h"
#include "adip_wire.h"

#include <string.h>

const adip_tool_t *adip_find_tool(const adip_device_t *dev, const char *id)
{
    uint16_t i;

    if (dev == NULL || id == NULL || dev->tools == NULL) {
        return NULL;
    }

    for (i = 0; i < dev->tool_count; i++) {
        if (strcmp(dev->tools[i].id, id) == 0) {
            return &dev->tools[i];
        }
    }

    /* Unknown means rejected. There is no fallback. */
    return NULL;
}

int adip_tool_index(const adip_device_t *dev, const adip_tool_t *tool)
{
    ptrdiff_t idx;

    if (dev == NULL || tool == NULL || dev->tools == NULL) {
        return -1;
    }

    idx = tool - dev->tools;
    if (idx < 0 || idx >= (ptrdiff_t)dev->tool_count) {
        return -1;
    }

    return (int)idx;
}

int adip_param_index(const adip_tool_t *tool, const char *name)
{
    uint8_t i;

    if (tool == NULL || name == NULL || tool->params == NULL) {
        return -1;
    }

    for (i = 0; i < tool->param_count; i++) {
        if (strcmp(tool->params[i].name, name) == 0) {
            return (int)i;
        }
    }

    return -1;
}

/*
 * One numeric argument, by the parameter name the declaration gives it.
 *
 * See the note in adip_wire.h on why this is by name. In short: `inv->args` is in
 * the order the sender encoded and each entry carries the parameter it belongs
 * to, so position is not meaning.
 */
bool adip_arg_num(const adip_invocation_t *inv, const char *name,
                  adip_num_t *out)
{
    uint8_t i;

    if (inv == NULL || name == NULL || out == NULL || inv->tool == NULL) {
        return false;
    }

    for (i = 0; i < inv->arg_count; i++) {
        const adip_param_t *p = &inv->tool->params[inv->args[i].index];

        /* Text arguments are the enum case and carry no number. Returning
         * false is correct: the caller asked for a value this argument does
         * not have, and a zero would look like one. */
        if (inv->args[i].is_text == 0u && strcmp(p->name, name) == 0) {
            *out = inv->args[i].num;
            return true;
        }
    }

    return false;
}
