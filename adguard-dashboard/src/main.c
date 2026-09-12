/**
 * @file main.c
 * AdGuard Home statistics dashboard for LVGL.
 *
 * The display comes from the @sgc daemon (LV_USE_SGC in lv_conf.h) and the data
 * from AdGuard Home's own API; the app itself only owns the layout. Nothing is
 * interactive: with no input device there is nothing to scroll or click, so the
 * poll loop is also the only thing that ever updates the screen.
 */

#include <errno.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "lvgl.h"

#include "agh.h"
#include "config.h"
#include "ui.h"

#define DEFAULT_CONFIG  "/etc/agh-dash/config.toml"
#define POLL_PERIOD_MS  1000 /**< how often libcurl is pumped and the deadline checked */
#define DRM_CARD        "/dev/dri/card0"

static volatile sig_atomic_t quit_flag;

static agh_config_t      cfg;
static agh_client_t *    client;
static agh_dashboard_t * ui;

/* The snapshot survives refreshes: a failed fetch keeps the last good numbers
 * on screen and only the status line changes. */
static agh_snapshot_t snapshot;

static uint32_t last_ok_tick;
static uint32_t last_attempt_tick;
static int      failures;
static bool     unauthorized;

static lv_display_t * g_disp;      /**< for the self-check (debug builds of it) */
static bool           self_check;  /**< --self-check: dump what is on screen, then exit */
static int            applied;     /**< successful refreshes so far */

static void on_signal(int sig)
{
    LV_UNUSED(sig);
    quit_flag = 1;
}

static uint32_t tick_cb(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint32_t)((uint64_t)t.tv_sec * 1000ULL + (uint64_t)t.tv_nsec / 1000000ULL);
}

static void delay_cb(uint32_t ms)
{
    struct timespec req = { .tv_sec = (time_t)(ms / 1000), .tv_nsec = (long)(ms % 1000) * 1000000L };
    while(nanosleep(&req, &req) == -1 && errno == EINTR) {
        /* keep sleeping the remainder */
    }
}

static lv_display_t * create_display(void)
{
#if LV_USE_SGC
    /* The @sgc daemon hands us a DRM lease (and nothing else: this app takes no
     * input device, so the daemon keeps them available to other clients). */
    lv_display_t * disp = lv_sgc_create();
    if(disp == NULL) {
        fprintf(stderr, "[agh-dash] no display - is the @sgc daemon running?\n");
    }
    return disp;
#else
    lv_display_t * disp = lv_linux_drm_create();
    if(disp == NULL) return NULL;
    if(lv_linux_drm_set_file(disp, DRM_CARD, -1) != LV_RESULT_OK) {
        fprintf(stderr, "[agh-dash] cannot open %s\n", DRM_CARD);
        lv_display_delete(disp);
        return NULL;
    }
    return disp;
#endif
}

static void poll_cb(lv_timer_t * t)
{
    LV_UNUSED(t);

    if(client == NULL) return;

    agh_fetch_result_t res;
    if(agh_client_pump(client, &snapshot, &res)) {
        if(res == AGH_FETCH_OK) {
            failures = 0;
            unauthorized = false;
            last_ok_tick = lv_tick_get();
            applied++;
            agh_ui_update(ui, &snapshot);
            fprintf(stderr, "[agh-dash] refreshed: %llu queries, %llu blocked\n",
                    (unsigned long long)snapshot.dns_queries, (unsigned long long)snapshot.blocked_filtering);
        }
        else {
            failures++;
            if(res == AGH_FETCH_UNAUTHORIZED) unauthorized = true;
            fprintf(stderr, "[agh-dash] refresh failed (%s)\n",
                    res == AGH_FETCH_UNAUTHORIZED ? "credentials rejected" : "fetch error");
        }
    }

    if(lv_tick_elaps(last_attempt_tick) >= (uint32_t)cfg.refresh_secs * 1000U) {
        last_attempt_tick = lv_tick_get();
        agh_client_refresh(client);
    }
}

/** Keeps the header honest about how old the numbers are. */
static void status_cb(lv_timer_t * t)
{
    LV_UNUSED(t);

    /* --self-check: let two refreshes land, report the pixels, then leave. */
    if(self_check && applied >= 2) {
        agh_ui_self_check(ui, g_disp);
        quit_flag = 1;
        return;
    }

    char text[128];

    if(unauthorized) {
        agh_ui_status(ui, "credentials rejected - check the config", true);
        return;
    }
    if(last_ok_tick == 0) {
        agh_ui_status(ui, failures ? "cannot reach AdGuard Home" : "waiting for data", failures ? true : false);
        return;
    }

    uint32_t age = lv_tick_elaps(last_ok_tick) / 1000U;
    if(failures == 0) {
        snprintf(text, sizeof(text), "updated %us ago - every %ds", (unsigned)age, cfg.refresh_secs);
        agh_ui_status(ui, text, false);
    }
    else {
        snprintf(text, sizeof(text), "stale - %d failed fetch(es), last data %us ago", failures, (unsigned)age);
        agh_ui_status(ui, text, true);
    }
}

static void usage(const char * argv0)
{
    printf("usage: %s [--config <path>] [--self-check] [--help]\n"
           "  --config <path>   TOML configuration (default %s)\n"
           "  --self-check      after two refreshes, report the on-screen chart areas and\n"
           "                    pixel counts, then exit (debug aid)\n", argv0, DEFAULT_CONFIG);
}

int main(int argc, char ** argv)
{
    const char * config_path = DEFAULT_CONFIG;

    for(int i = 1; i < argc; i++) {
        if(strcmp(argv[i], "--config") == 0 && i + 1 < argc) {
            config_path = argv[++i];
        }
        else if(strcmp(argv[i], "--self-check") == 0) {
            self_check = true;
        }
        else if(strcmp(argv[i], "--help") == 0 || strcmp(argv[i], "-h") == 0) {
            usage(argv[0]);
            return 0;
        }
        else {
            fprintf(stderr, "[agh-dash] unknown argument '%s'\n", argv[i]);
            usage(argv[0]);
            return 2;
        }
    }

    char err[256] = {0};
    if(agh_config_load(config_path, &cfg, err, sizeof(err)) != 0) {
        fprintf(stderr, "[agh-dash] configuration error: %s\n", err);
        return 1;
    }
    fprintf(stderr, "[agh-dash] %s as '%s', refresh every %ds\n", cfg.base_url, cfg.username, cfg.refresh_secs);

    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);

    lv_init();
    lv_tick_set_cb(tick_cb);
    lv_delay_set_cb(delay_cb);

    lv_display_t * disp = create_display();
    if(disp == NULL) {
        lv_deinit();
        return 1;
    }
    g_disp = disp;
    fprintf(stderr, "[agh-dash] display %" LV_PRId32 "x%" LV_PRId32 "\n",
            lv_display_get_horizontal_resolution(disp), lv_display_get_vertical_resolution(disp));

    ui = agh_ui_create(disp, cfg.base_url);
    agh_ui_status(ui, "waiting for data", false);

    client = agh_client_create(&cfg);
    if(client == NULL) {
        agh_ui_fatal(ui, "cannot initialise libcurl");
        lv_display_delete(disp);
        lv_deinit();
        return 1;
    }

    last_attempt_tick = lv_tick_get();
    agh_client_refresh(client);

    lv_timer_create(poll_cb, POLL_PERIOD_MS, NULL);
    lv_timer_create(status_cb, 1000, NULL);

    while(!quit_flag) {
        uint32_t time_till_next = lv_timer_handler();
        lv_delay_ms(time_till_next ? time_till_next : 1);
    }

    fprintf(stderr, "[agh-dash] exiting\n");
    agh_client_destroy(client);
    lv_display_delete(disp);
    lv_deinit();
    return 0;
}
