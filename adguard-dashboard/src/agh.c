/**
 * @file agh.c
 * AdGuard Home API client.
 *
 * Both requests are driven by a libcurl multi handle, pumped from the LVGL
 * timer: nothing here blocks the UI thread, and the credentials never appear in
 * log output.
 */

#include "agh.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <cJSON.h>
#include <curl/curl.h>

typedef struct {
    char * data;
    size_t len;
    size_t cap;
} body_t;

typedef struct {
    CURL *  easy;
    body_t  body;
    long    http_code;
    CURLcode curl_code; /**< why the transfer ended, when it did not succeed */
    bool    done;
} request_t;

struct agh_client {
    CURLM *   multi;
    request_t stats;   /**< /control/stats */
    request_t status;  /**< /control/status */
    bool      in_flight;
};

/**********************
 *   RESPONSE BUFFER
 **********************/

static bool body_reserve(body_t * b, size_t need)
{
    if(need <= b->cap) return true;

    size_t cap = b->cap ? b->cap : 4096;
    while(cap < need) cap *= 2;

    char * p = realloc(b->data, cap);
    if(p == NULL) return false;

    b->data = p;
    b->cap = cap;
    return true;
}

static size_t write_cb(char * ptr, size_t size, size_t nmemb, void * userdata)
{
    body_t * b = userdata;
    size_t n = size * nmemb;

    if(!body_reserve(b, b->len + n + 1)) return 0; /* aborts the transfer */

    memcpy(b->data + b->len, ptr, n);
    b->len += n;
    b->data[b->len] = '\0';
    return n;
}

static void body_reset(body_t * b)
{
    b->len = 0;
    if(b->data) b->data[0] = '\0';
}

static void body_free(body_t * b)
{
    free(b->data);
    b->data = NULL;
    b->len = b->cap = 0;
}

/**********************
 *   JSON HELPERS
 **********************/

static uint64_t j_u64(const cJSON * o, const char * key)
{
    const cJSON * i = cJSON_GetObjectItemCaseSensitive(o, key);
    return (i && cJSON_IsNumber(i)) ? (uint64_t)i->valuedouble : 0;
}

static void j_str(const cJSON * o, const char * key, char * dst, size_t cap)
{
    const cJSON * i = cJSON_GetObjectItemCaseSensitive(o, key);
    if(i && cJSON_IsString(i) && i->valuestring) snprintf(dst, cap, "%s", i->valuestring);
}

/**
 * Copy the NEWEST entries of an integer array (one statistics series).
 *
 * AdGuard sends the whole interval oldest first - 168 hourly buckets for 7 days
 * - so keeping the head would chart the oldest, mostly idle hours and drop what
 * just happened. Returns how many entries were kept; *total_out gets the full
 * length, for the "statistics interval" row.
 */
static size_t j_u32arr_tail(const cJSON * o, const char * key, uint32_t * dst, size_t cap, size_t * total_out)
{
    const cJSON * arr = cJSON_GetObjectItemCaseSensitive(o, key);
    if(arr == NULL || !cJSON_IsArray(arr)) return 0;

    size_t len = (size_t)cJSON_GetArraySize(arr);
    if(total_out) *total_out = len;

    size_t keep = len < cap ? len : cap;
    size_t skip = len - keep;
    size_t n = 0;
    size_t idx = 0;
    const cJSON * v = NULL;
    cJSON_ArrayForEach(v, arr) {
        if(idx++ < skip) continue;
        dst[n++] = (v && cJSON_IsNumber(v)) ? (uint32_t)v->valuedouble : 0;
    }
    return n;
}

/** Copy a top list; AdGuard sends [{ "<key>": <number> }, ...]. */
static size_t j_top(const cJSON * o, const char * key, agh_top_t * dst, size_t cap)
{
    const cJSON * arr = cJSON_GetObjectItemCaseSensitive(o, key);
    if(arr == NULL || !cJSON_IsArray(arr)) return 0;

    size_t n = 0;
    const cJSON * el = NULL;
    cJSON_ArrayForEach(el, arr) {
        if(n >= cap) break;

        const cJSON * first = el ? el->child : NULL; /* the single "<key>": value pair */
        if(first == NULL || first->string == NULL || !cJSON_IsNumber(first)) continue;

        snprintf(dst[n].key, sizeof(dst[n].key), "%s", first->string);
        dst[n].value = first->valuedouble;
        n++;
    }
    return n;
}

static bool parse_stats(const char * json, size_t len, agh_snapshot_t * s)
{
    if(json == NULL || len == 0) return false;

    cJSON * root = cJSON_ParseWithLength(json, len);
    if(root == NULL) return false;

    s->dns_queries = j_u64(root, "num_dns_queries");
    s->blocked_filtering = j_u64(root, "num_blocked_filtering");
    s->blocked_safebrowsing = j_u64(root, "num_replaced_safebrowsing");
    s->blocked_safesearch = j_u64(root, "num_replaced_safesearch");
    s->blocked_parental = j_u64(root, "num_replaced_parental");

    const cJSON * avg = cJSON_GetObjectItemCaseSensitive(root, "avg_processing_time");
    s->avg_processing_ms = (avg && cJSON_IsNumber(avg)) ? avg->valuedouble * 1000.0 : 0.0;

    j_str(root, "time_units", s->time_units, sizeof(s->time_units));

    /* The four series can differ in length; a missing tail is simply 0. */
    memset(s->dns_queries_series, 0, sizeof(s->dns_queries_series));
    memset(s->blocked_filtering_series, 0, sizeof(s->blocked_filtering_series));
    memset(s->blocked_safebrowsing_series, 0, sizeof(s->blocked_safebrowsing_series));
    memset(s->blocked_parental_series, 0, sizeof(s->blocked_parental_series));

    size_t total = 0;
    size_t n_dns = j_u32arr_tail(root, "dns_queries", s->dns_queries_series, AGH_MAX_BUCKETS, &total);
    size_t n_blk = j_u32arr_tail(root, "blocked_filtering", s->blocked_filtering_series, AGH_MAX_BUCKETS, NULL);
    size_t n_sb = j_u32arr_tail(root, "replaced_safebrowsing", s->blocked_safebrowsing_series, AGH_MAX_BUCKETS, NULL);
    size_t n_par = j_u32arr_tail(root, "replaced_parental", s->blocked_parental_series, AGH_MAX_BUCKETS, NULL);
    s->series_len = total;

    size_t buckets = n_dns;
    if(n_blk > buckets) buckets = n_blk;
    if(n_sb > buckets) buckets = n_sb;
    if(n_par > buckets) buckets = n_par;
    s->buckets = buckets;

    s->n_clients = j_top(root, "top_clients", s->clients, AGH_MAX_TOP);
    s->n_queried = j_top(root, "top_queried_domains", s->queried, AGH_MAX_TOP);
    s->n_blocked = j_top(root, "top_blocked_domains", s->blocked, AGH_MAX_TOP);

    cJSON_Delete(root);
    return true;
}

static bool parse_status(const char * json, size_t len, agh_snapshot_t * s)
{
    if(json == NULL || len == 0) return false;

    cJSON * root = cJSON_ParseWithLength(json, len);
    if(root == NULL) return false;

    const cJSON * prot = cJSON_GetObjectItemCaseSensitive(root, "protection_enabled");
    if(prot && cJSON_IsBool(prot)) s->protection_enabled = cJSON_IsTrue(prot);
    j_str(root, "version", s->version, sizeof(s->version));

    cJSON_Delete(root);
    return true;
}

/**********************
 *   CLIENT
 **********************/

static void set_common_opts(CURL * easy, const agh_config_t * cfg, const char * url, body_t * body)
{
    curl_easy_setopt(easy, CURLOPT_URL, url);
    curl_easy_setopt(easy, CURLOPT_WRITEFUNCTION, write_cb);
    curl_easy_setopt(easy, CURLOPT_WRITEDATA, body);
    curl_easy_setopt(easy, CURLOPT_HTTPAUTH, (long)CURLAUTH_BASIC);
    curl_easy_setopt(easy, CURLOPT_USERNAME, cfg->username);
    curl_easy_setopt(easy, CURLOPT_PASSWORD, cfg->password);
    /* Wall-clock budget for the whole transfer. The transfers are advanced from
     * the LVGL timer, so a slow frame (the first full-screen paint takes ~2 s)
     * delays the pump: the timeout has to tolerate that, while still catching a
     * server that stops answering. */
    curl_easy_setopt(easy, CURLOPT_TIMEOUT_MS, (long)cfg->timeout_ms);
    curl_easy_setopt(easy, CURLOPT_CONNECTTIMEOUT_MS, 2000L);
    curl_easy_setopt(easy, CURLOPT_NOSIGNAL, 1L); /* the UI thread has no signal budget */
    curl_easy_setopt(easy, CURLOPT_FOLLOWLOCATION, 1L);
    curl_easy_setopt(easy, CURLOPT_TCP_KEEPALIVE, 1L);
    curl_easy_setopt(easy, CURLOPT_USERAGENT, "agh-dash/1.0");
    curl_easy_setopt(easy, CURLOPT_SSL_VERIFYPEER, cfg->verify_tls ? 1L : 0L);
    curl_easy_setopt(easy, CURLOPT_SSL_VERIFYHOST, cfg->verify_tls ? 2L : 0L);
}

agh_client_t * agh_client_create(const agh_config_t * cfg)
{
    if(curl_global_init(CURL_GLOBAL_DEFAULT) != CURLE_OK) {
        fprintf(stderr, "[agh] libcurl initialisation failed\n");
        return NULL;
    }

    agh_client_t * c = calloc(1, sizeof(*c));
    if(c == NULL) return NULL;

    c->multi = curl_multi_init();
    c->stats.easy = curl_easy_init();
    c->status.easy = curl_easy_init();
    if(c->multi == NULL || c->stats.easy == NULL || c->status.easy == NULL) {
        fprintf(stderr, "[agh] cannot create the curl handles\n");
        agh_client_destroy(c);
        return NULL;
    }

    char url[256];
    snprintf(url, sizeof(url), "%s/control/stats", cfg->base_url);
    set_common_opts(c->stats.easy, cfg, url, &c->stats.body);

    snprintf(url, sizeof(url), "%s/control/status", cfg->base_url);
    set_common_opts(c->status.easy, cfg, url, &c->status.body);

    return c;
}

void agh_client_destroy(agh_client_t * c)
{
    if(c == NULL) return;

    if(c->multi != NULL) {
        if(c->stats.easy != NULL) {
            curl_multi_remove_handle(c->multi, c->stats.easy);
            curl_easy_cleanup(c->stats.easy);
        }
        if(c->status.easy != NULL) {
            curl_multi_remove_handle(c->multi, c->status.easy);
            curl_easy_cleanup(c->status.easy);
        }
        curl_multi_cleanup(c->multi);
    }

    body_free(&c->stats.body);
    body_free(&c->status.body);
    free(c);
    curl_global_cleanup();
}

void agh_client_refresh(agh_client_t * c)
{
    if(c == NULL || c->in_flight) return;

    body_reset(&c->stats.body);
    body_reset(&c->status.body);
    c->stats.done = c->status.done = false;
    c->stats.http_code = c->status.http_code = 0;
    c->stats.curl_code = c->status.curl_code = CURLE_OK;

    curl_multi_add_handle(c->multi, c->stats.easy);
    curl_multi_add_handle(c->multi, c->status.easy);
    c->in_flight = true;
}

bool agh_client_pump(agh_client_t * c, agh_snapshot_t * out, agh_fetch_result_t * result)
{
    if(c == NULL || !c->in_flight) return false;

    int running = 0;
    CURLMcode mc = curl_multi_perform(c->multi, &running);
    if(mc != CURLM_OK) fprintf(stderr, "[agh] curl_multi_perform: %s\n", curl_multi_strerror(mc));

    int queued = 0;
    CURLMsg * msg;
    while((msg = curl_multi_info_read(c->multi, &queued)) != NULL) {
        if(msg->msg != CURLMSG_DONE) continue;

        CURL * easy = msg->easy_handle;
        long code = 0;
        curl_easy_getinfo(easy, CURLINFO_RESPONSE_CODE, &code);

        if(easy == c->stats.easy) {
            c->stats.done = true;
            c->stats.http_code = code;
            c->stats.curl_code = msg->data.result;
        }
        else if(easy == c->status.easy) {
            c->status.done = true;
            c->status.http_code = code;
            c->status.curl_code = msg->data.result;
        }
    }

    /* Each request carries its own timeout, so both always finish. */
    if(!c->stats.done || !c->status.done) return false;

    c->in_flight = false;
    curl_multi_remove_handle(c->multi, c->stats.easy);
    curl_multi_remove_handle(c->multi, c->status.easy);

    agh_fetch_result_t res;
    if(c->stats.curl_code != CURLE_OK) {
        fprintf(stderr, "[agh] /control/stats: %s (curl code %d)\n",
                curl_easy_strerror(c->stats.curl_code), (int)c->stats.curl_code);
        res = AGH_FETCH_FAILED;
    }
    else if(c->stats.http_code == 401 || c->stats.http_code == 403) {
        res = AGH_FETCH_UNAUTHORIZED;
    }
    else if(c->stats.http_code != 200) {
        fprintf(stderr, "[agh] /control/stats returned HTTP %ld\n", c->stats.http_code);
        res = AGH_FETCH_FAILED;
    }
    else if(!parse_stats(c->stats.body.data, c->stats.body.len, out)) {
        fprintf(stderr, "[agh] cannot parse the statistics response\n");
        res = AGH_FETCH_FAILED;
    }
    else {
        res = AGH_FETCH_OK;
    }

    /* A status failure only costs the protection/version rows. */
    if(c->status.curl_code != CURLE_OK) {
        fprintf(stderr, "[agh] /control/status: %s (curl code %d)\n",
                curl_easy_strerror(c->status.curl_code), (int)c->status.curl_code);
    }
    else if(c->status.http_code == 200) {
        parse_status(c->status.body.data, c->status.body.len, out);
    }
    else {
        fprintf(stderr, "[agh] /control/status returned HTTP %ld\n", c->status.http_code);
    }

    *result = res;
    return true;
}
