/**
 * @file ui.c
 * Layout (mirrors the AdGuard Home statistics page):
 *
 *   header   : "AdGuard Home" + source, and the refresh status on the right
 *   row 1    : 4 charts - DNS queries / blocked by filters / blocked
 *              malware-phishing / blocked adult websites
 *   row 2    : 2 tables - general statistics | top clients
 *   row 3    : 2 tables - top queried domains | top blocked domains
 *
 * The display has no input device, so every container is non-scrollable and the
 * tables are sized to the rows they can show.
 */

#include "ui.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "lvgl.h"

#define AGH_UI_CHARTS   4
#define AGH_UI_TOP_ROWS 9  /**< data rows per top-list table (fixed: no scrolling) */
#define AGH_UI_GEN_ROWS 7  /**< data rows in the general statistics table */

#define COL_PAGE     lv_color_hex(0x0F1419)
#define COL_CARD     lv_color_hex(0x19212A)
#define COL_TEXT     lv_color_hex(0xE6EDF3)
#define COL_MUTED    lv_color_hex(0x8B98A5)
#define COL_GRID     lv_color_hex(0x2B3644)
#define COL_OK       lv_color_hex(0x4FD1A5)
#define COL_BAD      lv_color_hex(0xE5534B)
#define COL_PROT_ON  lv_color_hex(0x2F9E63) /* protection badge: filtering active */
#define COL_PROT_OFF lv_color_hex(0xD5473F) /* protection badge: filtering off */
#define COL_ON_BADGE lv_color_hex(0xF2F6FA) /* text on the badge */

#define COL_QUERIES  lv_color_hex(0x4FA3E3)
#define COL_FILTERED lv_color_hex(0xE8843C)
#define COL_MALWARE  lv_color_hex(0xA86FE0)
#define COL_ADULT    lv_color_hex(0x4FD1A5)

struct agh_dashboard {
    lv_obj_t * status_lbl;
    lv_obj_t * protection_box;   /**< header badge: green enabled / red disabled */
    lv_obj_t * protection_lbl;
    lv_obj_t * footer_version;   /**< footer: "AdGuard Home vX.Y.Z" */
    lv_obj_t * footer_source;
    lv_obj_t * probe_title;      /**< a section title, measured by the self-check */

    lv_obj_t * chart[AGH_UI_CHARTS];
    lv_chart_series_t * series[AGH_UI_CHARTS];
    lv_obj_t * value[AGH_UI_CHARTS];
    lv_obj_t * empty[AGH_UI_CHARTS]; /**< "no activity" hint, shown when a series is all zeros */
    lv_color_t chart_color[AGH_UI_CHARTS];

    lv_obj_t * table_general;
    lv_obj_t * table_clients;
    lv_obj_t * table_queried;
    lv_obj_t * table_blocked;
};

/**********************
 *   SMALL HELPERS
 **********************/

/** 1234567 -> "1,234,567" (LVGL's snprintf has no %' grouping). */
static void group_u64(uint64_t v, char * out, size_t cap)
{
    char raw[24];
    snprintf(raw, sizeof(raw), "%llu", (unsigned long long)v);

    size_t len = strlen(raw);
    size_t out_i = 0;
    for(size_t i = 0; i < len && out_i + 2 < cap; i++) {
        if(i > 0 && ((len - i) % 3) == 0) out[out_i++] = ',';
        out[out_i++] = raw[i];
    }
    out[out_i] = '\0';
}

/** "12.3%" without floating point formatting (LVGL's snprintf has no %f). */
static void permille(uint64_t part, uint64_t total, char * out, size_t cap)
{
    uint64_t p10 = total ? (part * 1000ULL) / total : 0; /* tenths of a percent */
    snprintf(out, cap, "%llu.%llu%%", (unsigned long long)(p10 / 10), (unsigned long long)(p10 % 10));
}

static void style_card(lv_obj_t * obj)
{
    lv_obj_set_style_bg_color(obj, COL_CARD, 0);
    lv_obj_set_style_border_width(obj, 0, 0);
    lv_obj_set_style_radius(obj, 10, 0);
    lv_obj_set_style_pad_all(obj, 12, 0);
    lv_obj_set_style_pad_gap(obj, 6, 0);
    lv_obj_remove_flag(obj, LV_OBJ_FLAG_SCROLLABLE);
}

static lv_obj_t * make_row(lv_obj_t * parent, int32_t height_pct)
{
    lv_obj_t * row = lv_obj_create(parent);
    lv_obj_remove_style_all(row);
    lv_obj_set_width(row, lv_pct(100));
    if(height_pct > 0) lv_obj_set_height(row, lv_pct(height_pct));
    else lv_obj_set_flex_grow(row, 1);
    lv_obj_set_flex_flow(row, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_gap(row, 12, 0);
    lv_obj_remove_flag(row, LV_OBJ_FLAG_SCROLLABLE);
    return row;
}

static lv_obj_t * make_label(lv_obj_t * parent, const char * text, const lv_font_t * font, lv_color_t color)
{
    lv_obj_t * lbl = lv_label_create(parent);
    lv_label_set_text(lbl, text);
    lv_obj_set_style_text_font(lbl, font, 0);
    lv_obj_set_style_text_color(lbl, color, 0);
    return lbl;
}

/** A chart card: metric name, running total, and the chart itself. */
static lv_obj_t * make_chart_card(lv_obj_t * parent, const char * title, lv_color_t color,
                                  lv_obj_t ** chart_out, lv_chart_series_t ** series_out, lv_obj_t ** value_out,
                                  lv_obj_t ** empty_out)
{
    lv_obj_t * card = lv_obj_create(parent);
    style_card(card);
    lv_obj_set_flex_grow(card, 1);
    lv_obj_set_height(card, lv_pct(100));
    lv_obj_set_flex_flow(card, LV_FLEX_FLOW_COLUMN);

    /* Section titles are a step larger than the content they head (16 px cells),
     * so a card reads as title + body rather than one block of text. */
    lv_obj_t * chart_title = make_label(card, title, &lv_font_montserrat_20, COL_MUTED);
    lv_obj_set_style_pad_bottom(chart_title, 8, 0);
    *value_out = make_label(card, "0", &lv_font_montserrat_28, COL_TEXT);

    lv_obj_t * chart = lv_chart_create(card);
    lv_obj_set_width(chart, lv_pct(100));
    lv_obj_set_flex_grow(chart, 1);
    lv_chart_set_type(chart, LV_CHART_TYPE_BAR);
    lv_chart_set_point_count(chart, 24);
    lv_chart_set_axis_range(chart, LV_CHART_AXIS_PRIMARY_Y, 0, 10);
    lv_chart_set_div_line_count(chart, 2, 0);
    lv_obj_set_style_bg_opa(chart, LV_OPA_TRANSP, 0);
    lv_obj_set_style_border_width(chart, 0, 0);
    lv_obj_set_style_line_color(chart, COL_GRID, LV_PART_MAIN);
    lv_obj_set_style_pad_all(chart, 0, 0);
    lv_obj_remove_flag(chart, LV_OBJ_FLAG_SCROLLABLE);

    *series_out = lv_chart_add_series(chart, color, LV_CHART_AXIS_PRIMARY_Y);
    lv_obj_set_style_bg_color(chart, color, LV_PART_ITEMS); /* bar colour */

    /* A series that is all zeros draws nothing; say so instead of leaving the
     * card looking broken. */
    lv_obj_t * empty = make_label(chart, "no activity in this interval", &lv_font_montserrat_14, COL_MUTED);
    lv_obj_center(empty);
    lv_obj_add_flag(empty, LV_OBJ_FLAG_HIDDEN);
    *empty_out = empty;

    *chart_out = chart;
    return card;
}

/** A table card: title plus a fixed-size 2-column table. */
/** Size the two columns from the card's real width: the numbers column takes a
 *  capped slice, the label column the rest, so the grid spans the whole card
 *  instead of a hardcoded pixel width. Re-run on every resize. */
static void table_fit(lv_obj_t * table)
{
    int32_t w = lv_obj_get_content_width(table);
    if(w <= 0) return;

    int32_t value_w = w * 26 / 100;
    if(value_w < 110) value_w = 110;
    if(value_w > 260) value_w = 260;
    int32_t label_w = w - value_w;
    if(label_w < 100) { /* very narrow card: split evenly */
        label_w = w / 2;
        value_w = w - label_w;
    }
    if(lv_table_get_column_width(table, 0) == label_w &&
       lv_table_get_column_width(table, 1) == value_w) {
        return; /* already right - keeps the size event from re-entering */
    }
    lv_table_set_column_width(table, 0, label_w);
    lv_table_set_column_width(table, 1, value_w);
}

static void table_fit_cb(lv_event_t * e)
{
    table_fit(lv_event_get_target(e));
}

static lv_obj_t * make_table_card(lv_obj_t * parent, const char * title, const char * col0, const char * col1,
                                  uint32_t rows, const lv_font_t * font,
                                  lv_obj_t ** title_out)
{
    lv_obj_t * card = lv_obj_create(parent);
    style_card(card);
    lv_obj_set_flex_grow(card, 1);
    lv_obj_set_height(card, lv_pct(100));
    lv_obj_set_flex_flow(card, LV_FLEX_FLOW_COLUMN);

    lv_obj_t * title_lbl = make_label(card, title, &lv_font_montserrat_20, COL_MUTED);
    /* a little more air between a section title and its content than the 6 px card gap */
    lv_obj_set_style_pad_bottom(title_lbl, 8, 0);
    if(title_out) *title_out = title_lbl;

    lv_obj_t * table = lv_table_create(card);
    lv_obj_set_width(table, lv_pct(100));
    lv_obj_set_flex_grow(table, 1);
    lv_table_set_column_count(table, 2);
    lv_table_set_row_count(table, rows);
    lv_table_set_column_width(table, 0, 200); /* replaced by table_fit() on the first layout */
    lv_table_set_column_width(table, 1, 120);
    lv_obj_add_event_cb(table, table_fit_cb, LV_EVENT_SIZE_CHANGED, NULL);
    lv_obj_set_style_bg_opa(table, LV_OPA_TRANSP, 0);
    lv_obj_set_style_border_width(table, 0, 0);
    lv_obj_set_style_text_font(table, font, LV_PART_ITEMS);
    lv_obj_set_style_text_color(table, COL_TEXT, LV_PART_ITEMS);
    lv_obj_set_style_bg_opa(table, LV_OPA_TRANSP, LV_PART_ITEMS);
    lv_obj_set_style_border_side(table, LV_BORDER_SIDE_BOTTOM, LV_PART_ITEMS);
    lv_obj_set_style_border_color(table, COL_GRID, LV_PART_ITEMS);
    lv_obj_set_style_border_width(table, 1, LV_PART_ITEMS);
    lv_obj_set_style_pad_all(table, 6, LV_PART_ITEMS);
    lv_obj_remove_flag(table, LV_OBJ_FLAG_SCROLLABLE);

    lv_table_set_cell_value(table, 0, 0, col0);
    lv_table_set_cell_value(table, 0, 1, col1);
    lv_table_set_cell_ctrl(table, 0, 0, LV_TABLE_CELL_CTRL_TEXT_CROP);
    lv_table_set_cell_ctrl(table, 0, 1, LV_TABLE_CELL_CTRL_TEXT_CROP);

    return table;
}

/**********************
 *   PUBLIC
 **********************/

agh_dashboard_t * agh_ui_create(lv_display_t * disp, const char * source)
{
    agh_dashboard_t * d = calloc(1, sizeof(*d));
    if(d == NULL) return NULL;

    lv_obj_t * scr = lv_display_get_screen_active(disp);
    lv_obj_set_style_bg_color(scr, COL_PAGE, 0);
    lv_obj_set_style_bg_opa(scr, LV_OPA_COVER, 0);
    lv_obj_set_flex_flow(scr, LV_FLEX_FLOW_COLUMN);
    lv_obj_set_style_pad_all(scr, 16, 0);
    lv_obj_set_style_pad_gap(scr, 12, 0);
    lv_obj_remove_flag(scr, LV_OBJ_FLAG_SCROLLABLE);

    /* header: title on the left, protection badge and refresh status on the right */
    lv_obj_t * head = make_row(scr, -1);
    lv_obj_set_flex_grow(head, 0);
    lv_obj_set_height(head, LV_SIZE_CONTENT);
    lv_obj_set_flex_align(head, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    make_label(head, "AdGuard Home statistics", &lv_font_montserrat_24, COL_TEXT);

    lv_obj_t * head_right = lv_obj_create(head);
    lv_obj_remove_style_all(head_right);
    lv_obj_set_size(head_right, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
    lv_obj_set_flex_flow(head_right, LV_FLEX_FLOW_ROW);
    lv_obj_set_style_pad_gap(head_right, 14, 0);
    lv_obj_set_flex_align(head_right, LV_FLEX_ALIGN_END, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    lv_obj_remove_flag(head_right, LV_OBJ_FLAG_SCROLLABLE);

    d->status_lbl = make_label(head_right, "starting", &lv_font_montserrat_16, COL_MUTED);

    /* ...and give the protection badge its own line, right under that header:
     * green when filtering is on, red when it is off. */
    lv_obj_t * badge_row = make_row(scr, -1);
    lv_obj_set_flex_grow(badge_row, 0);
    lv_obj_set_height(badge_row, LV_SIZE_CONTENT);
    lv_obj_set_flex_align(badge_row, LV_FLEX_ALIGN_START, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);

    d->protection_box = lv_obj_create(badge_row);
    lv_obj_remove_style_all(d->protection_box);
    lv_obj_set_size(d->protection_box, LV_SIZE_CONTENT, LV_SIZE_CONTENT);
    lv_obj_set_style_bg_opa(d->protection_box, LV_OPA_COVER, 0);
    lv_obj_set_style_bg_color(d->protection_box, COL_MUTED, 0);
    lv_obj_set_style_radius(d->protection_box, 8, 0);
    lv_obj_set_style_pad_hor(d->protection_box, 14, 0);
    lv_obj_set_style_pad_ver(d->protection_box, 6, 0);
    d->protection_lbl = make_label(d->protection_box, "Protection: unknown", &lv_font_montserrat_16,
                                   COL_ON_BADGE);

    /* row 1: the four charts */
    lv_obj_t * row1 = make_row(scr, 32);
    static const char * titles[AGH_UI_CHARTS] = {
        "DNS queries", "Blocked by filters", "Blocked malware / phishing", "Blocked adult websites"
    };
    /* lv_color_hex() is not a constant expression, so build this at run time. */
    const lv_color_t colors[AGH_UI_CHARTS] = {
        lv_color_hex(0x4FA3E3), lv_color_hex(0xE8843C), lv_color_hex(0xA86FE0), lv_color_hex(0x4FD1A5)
    };
    for(int i = 0; i < AGH_UI_CHARTS; i++) {
        d->chart_color[i] = colors[i];
        make_chart_card(row1, titles[i], colors[i], &d->chart[i], &d->series[i], &d->value[i], &d->empty[i]);
    }

    /* row 2: general statistics | top clients */
    lv_obj_t * row2 = make_row(scr, 30);
    d->table_general = make_table_card(row2, "General statistics", "Metric", "Value",
                                       AGH_UI_GEN_ROWS + 1, &lv_font_montserrat_16, &d->probe_title);
    d->table_clients = make_table_card(row2, "Top clients", "Client", "Requests",
                                       AGH_UI_TOP_ROWS + 1, &lv_font_montserrat_16, NULL);

    /* row 3: top queried domains | top blocked domains */
    lv_obj_t * row3 = make_row(scr, -1);
    d->table_queried = make_table_card(row3, "Top queried domains", "Domain", "Requests",
                                       AGH_UI_TOP_ROWS + 1, &lv_font_montserrat_16, NULL);
    d->table_blocked = make_table_card(row3, "Top blocked domains", "Domain", "Requests",
                                       AGH_UI_TOP_ROWS + 1, &lv_font_montserrat_16, NULL);

    /* footer: what this is and where it reads from */
    lv_obj_t * foot = make_row(scr, -1);
    lv_obj_set_flex_grow(foot, 0);
    lv_obj_set_height(foot, LV_SIZE_CONTENT);
    lv_obj_set_flex_align(foot, LV_FLEX_ALIGN_SPACE_BETWEEN, LV_FLEX_ALIGN_CENTER, LV_FLEX_ALIGN_CENTER);
    d->footer_version = make_label(foot, "AdGuard Home", &lv_font_montserrat_14, COL_MUTED);
    d->footer_source = make_label(foot, source ? source : "", &lv_font_montserrat_14, COL_MUTED);

    return d;
}

void agh_ui_status(agh_dashboard_t * d, const char * text, bool error)
{
    if(d == NULL || d->status_lbl == NULL) return;

    lv_label_set_text(d->status_lbl, text);
    lv_obj_set_style_text_color(d->status_lbl, error ? COL_BAD : COL_MUTED, 0);
}

void agh_ui_fatal(agh_dashboard_t * d, const char * text)
{
    agh_ui_status(d, text, true);
}

static void fill_general(agh_dashboard_t * d, const agh_snapshot_t * s)
{
    char v[64];
    char quota[32];
    uint64_t blocked_total = s->blocked_filtering + s->blocked_safebrowsing
                             + s->blocked_safesearch + s->blocked_parental;
    int row = 1;

    group_u64(s->dns_queries, v, sizeof(v));
    lv_table_set_cell_value(d->table_general, row, 0, "DNS queries");
    lv_table_set_cell_value(d->table_general, row++, 1, v);

    group_u64(s->blocked_filtering, v, sizeof(v));
    permille(s->blocked_filtering, s->dns_queries, quota, sizeof(quota));
    lv_table_set_cell_value(d->table_general, row, 0, "Blocked by filters");
    snprintf(v + strlen(v), sizeof(v) - strlen(v), "  (%s)", quota);
    lv_table_set_cell_value(d->table_general, row++, 1, v);

    group_u64(s->blocked_safebrowsing, v, sizeof(v));
    lv_table_set_cell_value(d->table_general, row, 0, "Blocked malware / phishing");
    lv_table_set_cell_value(d->table_general, row++, 1, v);

    group_u64(s->blocked_parental, v, sizeof(v));
    lv_table_set_cell_value(d->table_general, row, 0, "Blocked adult websites");
    lv_table_set_cell_value(d->table_general, row++, 1, v);

    group_u64(s->blocked_safesearch, v, sizeof(v));
    lv_table_set_cell_value(d->table_general, row, 0, "Blocked safe search");
    lv_table_set_cell_value(d->table_general, row++, 1, v);

    group_u64(blocked_total, v, sizeof(v));
    permille(blocked_total, s->dns_queries, quota, sizeof(quota));
    snprintf(v + strlen(v), sizeof(v) - strlen(v), "  (%s)", quota);
    lv_table_set_cell_value(d->table_general, row, 0, "Blocked total");
    lv_table_set_cell_value(d->table_general, row++, 1, v);

    uint64_t avg10 = (uint64_t)(s->avg_processing_ms * 10.0 + 0.5); /* tenths of a millisecond */
    snprintf(v, sizeof(v), "%llu.%llu ms", (unsigned long long)(avg10 / 10), (unsigned long long)(avg10 % 10));
    lv_table_set_cell_value(d->table_general, row, 0, "Average processing time");
    lv_table_set_cell_value(d->table_general, row++, 1, v);

    while(row <= AGH_UI_GEN_ROWS) {
        lv_table_set_cell_value(d->table_general, row, 0, "");
        lv_table_set_cell_value(d->table_general, row++, 1, "");
    }
}

static void fill_top(lv_obj_t * table, const agh_top_t * entries, size_t count)
{
    char v[32];

    for(size_t i = 0; i < AGH_UI_TOP_ROWS; i++) {
        uint32_t row = (uint32_t)i + 1;
        lv_table_set_cell_ctrl(table, row, 0, LV_TABLE_CELL_CTRL_TEXT_CROP);
        lv_table_set_cell_ctrl(table, row, 1, LV_TABLE_CELL_CTRL_TEXT_CROP);
        if(i < count) {
            lv_table_set_cell_value(table, row, 0, entries[i].key);
            group_u64((uint64_t)(entries[i].value + 0.5), v, sizeof(v));
            lv_table_set_cell_value(table, row, 1, v);
        }
        else {
            lv_table_set_cell_value(table, row, 0, "");
            lv_table_set_cell_value(table, row, 1, "");
        }
    }
}

void agh_ui_update(agh_dashboard_t * d, const agh_snapshot_t * s)
{
    if(d == NULL || s == NULL) return;

    /* charts: one series each, full replacement every refresh */
    const uint32_t * src[AGH_UI_CHARTS] = {
        s->dns_queries_series, s->blocked_filtering_series,
        s->blocked_safebrowsing_series, s->blocked_parental_series
    };
    uint64_t totals[AGH_UI_CHARTS] = {
        s->dns_queries, s->blocked_filtering, s->blocked_safebrowsing, s->blocked_parental
    };
    static int32_t values[AGH_MAX_BUCKETS];

    uint32_t buckets = (uint32_t)(s->buckets ? s->buckets : 1);
    if(buckets > AGH_MAX_BUCKETS) buckets = AGH_MAX_BUCKETS;

    for(int i = 0; i < AGH_UI_CHARTS; i++) {
        int32_t max = 1;
        uint64_t sum = 0;
        for(uint32_t b = 0; b < buckets; b++) {
            int32_t v = (b < s->buckets) ? (int32_t)src[i][b] : 0;
            values[b] = v;
            sum += (uint64_t)v;
            if(v > max) max = v;
        }
        if(sum == 0) lv_obj_remove_flag(d->empty[i], LV_OBJ_FLAG_HIDDEN);
        else lv_obj_add_flag(d->empty[i], LV_OBJ_FLAG_HIDDEN);
        lv_chart_set_point_count(d->chart[i], buckets);
        lv_chart_set_series_values(d->chart[i], d->series[i], values, buckets);
        lv_chart_set_axis_range(d->chart[i], LV_CHART_AXIS_PRIMARY_Y, 0, max + max / 8 + 1);
        lv_chart_refresh(d->chart[i]);

        char v[32];
        group_u64(totals[i], v, sizeof(v));
        lv_label_set_text(d->value[i], v);
    }

    /* the badge in the header and the version in the footer */
    lv_obj_set_style_bg_color(d->protection_box, s->protection_enabled ? COL_PROT_ON : COL_PROT_OFF, 0);
    lv_label_set_text(d->protection_lbl, s->protection_enabled ? "Protection: enabled"
                                                               : "Protection: disabled");
    char ver[64];
    snprintf(ver, sizeof(ver), "AdGuard Home %s", s->version[0] ? s->version : "version unknown");
    lv_label_set_text(d->footer_version, ver);

    fill_general(d, s);
    fill_top(d->table_clients, s->clients, s->n_clients);
    fill_top(d->table_queried, s->queried, s->n_queried);
    fill_top(d->table_blocked, s->blocked, s->n_blocked);
}

/**********************
 *   SELF-CHECK (debug)
 **********************/

/** Count pixels of one colour inside an area of an XRGB8888 buffer. */
static uint32_t count_color(const uint8_t * px, uint32_t stride, const lv_area_t * a, lv_color_t col,
                            uint32_t buf_w, uint32_t buf_h)
{
    uint32_t n = 0;

    int32_t x1 = a->x1 < 0 ? 0 : a->x1;
    int32_t y1 = a->y1 < 0 ? 0 : a->y1;
    int32_t x2 = a->x2 >= (int32_t)buf_w ? (int32_t)buf_w - 1 : a->x2;
    int32_t y2 = a->y2 >= (int32_t)buf_h ? (int32_t)buf_h - 1 : a->y2;

    for(int32_t y = y1; y <= y2; y++) {
        const uint8_t * row = px + (size_t)y * stride;
        for(int32_t x = x1; x <= x2; x++) {
            const uint8_t * p = row + (size_t)x * 4; /* XRGB8888 little-endian: B, G, R, X */
            if(p[0] == col.blue && p[1] == col.green && p[2] == col.red) n++;
        }
    }
    return n;
}

void agh_ui_self_check(agh_dashboard_t * d, lv_display_t * disp)
{
    lv_draw_buf_t * buf = lv_display_get_buf_active(disp);
    if(buf == NULL || buf->data == NULL) {
        printf("[selfcheck] no active draw buffer\n");
        return;
    }

    uint32_t w = buf->header.w, h = buf->header.h, stride = buf->header.stride;
    printf("[selfcheck] buffer %ux%u stride=%u\n", (unsigned)w, (unsigned)h, (unsigned)stride);

    for(int i = 0; i < AGH_UI_CHARTS; i++) {
        lv_area_t a;
        lv_obj_get_coords(d->chart[i], &a);
        uint32_t bars = count_color((const uint8_t *)buf->data, stride, &a, d->chart_color[i], w, h);
        uint32_t series_pts = (uint32_t)lv_chart_get_point_count(d->chart[i]);
        uint32_t hint = count_color((const uint8_t *)buf->data, stride, &a, lv_color_hex(0x8B98A5), w, h);
        printf("[selfcheck] chart %d: %" LV_PRId32 "x%" LV_PRId32 " at (%" LV_PRId32 ",%" LV_PRId32 ")"
               " points=%u bars_px=%u hint_px=%u\n",
               i, a.x2 - a.x1 + 1, a.y2 - a.y1 + 1, a.x1, a.y1, (unsigned)series_pts, (unsigned)bars,
               (unsigned)hint);
    }

    /* gap between a section title and the table under it */
    {
        lv_area_t t, tb;
        lv_obj_get_coords(d->probe_title, &t);
        lv_obj_get_coords(d->table_general, &tb);
        printf("[selfcheck] title->table gap %" LV_PRId32 " px\n", tb.y1 - t.y2 - 1);
    }

    /* the general table's columns should now span the card */
    {
        int32_t c0 = lv_table_get_column_width(d->table_general, 0);
        int32_t c1 = lv_table_get_column_width(d->table_general, 1);
        printf("[selfcheck] general table columns %" LV_PRId32 " + %" LV_PRId32 " = %" LV_PRId32
               " (content width %" LV_PRId32 ")\n",
               c0, c1, c0 + c1, lv_obj_get_content_width(d->table_general));
    }

    /* a section title, to show the 20 px font next to 16 px cell text */
    {
        lv_area_t t;
        lv_obj_get_coords(d->probe_title, &t);
        printf("[selfcheck] section title %" LV_PRId32 "x%" LV_PRId32 " text=\"%s\"\n",
               t.x2 - t.x1 + 1, t.y2 - t.y1 + 1, lv_label_get_text(d->probe_title));
    }

    /* the protection badge and the footer version */
    {
        lv_area_t b;
        lv_obj_get_coords(d->protection_box, &b);
        uint32_t box = count_color((const uint8_t *)buf->data, stride, &b,
                                   lv_obj_get_style_bg_color(d->protection_box, 0), w, h);
        lv_color_t bg = lv_obj_get_style_bg_color(d->protection_box, 0);
        printf("[selfcheck] protection box %" LV_PRId32 "x%" LV_PRId32 " at (%" LV_PRId32 ",%" LV_PRId32 ")"
               " rgb=(%u,%u,%u) badge_px=%u text=\"%s\"\n",
               b.x2 - b.x1 + 1, b.y2 - b.y1 + 1, b.x1, b.y1,
               (unsigned)bg.red, (unsigned)bg.green, (unsigned)bg.blue, (unsigned)box,
               lv_label_get_text(d->protection_lbl));
    }
    {
        lv_area_t f;
        lv_obj_get_coords(d->footer_version, &f);
        uint32_t px = count_color((const uint8_t *)buf->data, stride, &f, lv_color_hex(0x8B98A5), w, h);
        printf("[selfcheck] footer version %" LV_PRId32 "x%" LV_PRId32 " at (%" LV_PRId32 ",%" LV_PRId32 ")"
               " text_px=%u text=\"%s\"\n", f.x2 - f.x1 + 1, f.y2 - f.y1 + 1, f.x1, f.y1, (unsigned)px,
               lv_label_get_text(d->footer_version));
    }

    /* Control: text pixels in the general statistics table, to prove the read-back works. */
    lv_area_t t;
    lv_obj_get_coords(d->table_general, &t);
    uint32_t text = count_color((const uint8_t *)buf->data, stride, &t, lv_color_hex(0xE6EDF3), w, h);
    printf("[selfcheck] general table %" LV_PRId32 "x%" LV_PRId32 " text_px=%u\n",
           t.x2 - t.x1 + 1, t.y2 - t.y1 + 1, (unsigned)text);
}
