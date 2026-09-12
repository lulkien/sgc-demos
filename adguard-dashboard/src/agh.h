/**
 * @file agh.h
 * AdGuard Home statistics client: fetches /control/status and /control/stats
 * asynchronously (libcurl multi) and parses them (cJSON) into a snapshot.
 */

#ifndef AGH_DASH_AGH_H
#define AGH_DASH_AGH_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "config.h"

/**
 * Chart buckets kept. AdGuard returns the whole configured interval (e.g. 168
 * hourly buckets for 7 days) oldest first, so the tail is what a dashboard
 * shows: the recent past, 24 bars wide.
 */
#define AGH_MAX_BUCKETS 24
/** Entries kept from each top list - only what the dashboard can show. */
#define AGH_MAX_TOP 10

typedef struct {
    char   key[96];   /**< domain, client address or upstream */
    double value;     /**< hits, or seconds for an average-time list */
} agh_top_t;

typedef struct {
    /* summary counters (AdGuard's statistics interval; see time_units) */
    uint64_t dns_queries;
    uint64_t blocked_filtering;
    uint64_t blocked_safebrowsing; /**< "blocked malware/phishing" */
    uint64_t blocked_safesearch;
    uint64_t blocked_parental;     /**< "blocked adult websites" */
    double   avg_processing_ms;

    /** Bucket size of the series: "hours" or "days". */
    char     time_units[8];
    size_t   series_len;           /**< how many buckets the server sent in total */
    size_t   buckets;              /**< valid entries kept in the four series (the newest) */
    uint32_t dns_queries_series[AGH_MAX_BUCKETS];
    uint32_t blocked_filtering_series[AGH_MAX_BUCKETS];
    uint32_t blocked_safebrowsing_series[AGH_MAX_BUCKETS];
    uint32_t blocked_parental_series[AGH_MAX_BUCKETS];

    size_t   n_clients;            /**< valid entries in clients[] */
    size_t   n_queried;            /**< valid entries in queried[] */
    size_t   n_blocked;            /**< valid entries in blocked[] */
    agh_top_t clients[AGH_MAX_TOP];
    agh_top_t queried[AGH_MAX_TOP];
    agh_top_t blocked[AGH_MAX_TOP];

    /* /control/status */
    bool     protection_enabled;
    char     version[32];
} agh_snapshot_t;

typedef enum {
    AGH_FETCH_OK,           /**< a fresh snapshot was parsed */
    AGH_FETCH_FAILED,       /**< network/protocol/parse failure */
    AGH_FETCH_UNAUTHORIZED, /**< AdGuard refused the credentials (401/403) */
} agh_fetch_result_t;

typedef struct agh_client agh_client_t;

/** Create the client (initialises libcurl once). Returns NULL on failure. */
agh_client_t * agh_client_create(const agh_config_t * cfg);

void agh_client_destroy(agh_client_t * c);

/** Start refreshing both endpoints. No-op while a refresh is in flight. */
void agh_client_refresh(agh_client_t * c);

/**
 * Drive libcurl without blocking.
 *
 * @return true when a refresh finished (then *result says how it went)
 */
bool agh_client_pump(agh_client_t * c, agh_snapshot_t * out, agh_fetch_result_t * result);

#endif /*AGH_DASH_AGH_H*/
