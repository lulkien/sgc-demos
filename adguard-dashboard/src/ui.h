/**
 * @file ui.h
 * The dashboard itself: four statistics charts over two rows of tables,
 * mirroring AdGuard Home's own statistics page. Read-only - the display takes
 * no input, so nothing here is scrollable or interactive.
 */

#ifndef AGH_DASH_UI_H
#define AGH_DASH_UI_H

#include <stdbool.h>

#include "lvgl.h"

#include "agh.h"

typedef struct agh_dashboard agh_dashboard_t;

/**
 * Build the dashboard on a display.
 * @param disp   the display to draw on
 * @param source the AdGuard Home endpoint, shown in the header
 */
agh_dashboard_t * agh_ui_create(lv_display_t * disp, const char * source);

/** Refresh every widget from a snapshot (charts, counters and all tables). */
void agh_ui_update(agh_dashboard_t * d, const agh_snapshot_t * s);

/**
 * Set the header status text.
 * @param error true to draw it as a warning
 */
void agh_ui_status(agh_dashboard_t * d, const char * text, bool error);

/** Something went wrong before any data arrived (e.g. bad config). */
void agh_ui_fatal(agh_dashboard_t * d, const char * text);

/**
 * Debug aid: read the display's active buffer back and report, per chart, its
 * on-screen size and how many pixels carry that chart's series colour (plus a
 * table's text pixels as a control). Answers "is the chart actually drawn?"
 * without needing to look at the screen.
 */
void agh_ui_self_check(agh_dashboard_t * d, lv_display_t * disp);

#endif /*AGH_DASH_UI_H*/
