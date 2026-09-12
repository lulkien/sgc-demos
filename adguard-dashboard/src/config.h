/**
 * @file config.h
 * Configuration of the AdGuard Home dashboard, read from a TOML file
 * (/etc/agh-dash/config.toml by default).
 */

#ifndef AGH_DASH_CONFIG_H
#define AGH_DASH_CONFIG_H

#include <stddef.h>

typedef struct {
    char base_url[192]; /**< AdGuard Home API base, e.g. "http://127.0.0.1:8080" (no trailing slash) */
    char username[64];  /**< basic auth user */
    char password[128]; /**< basic auth password */
    int refresh_secs;   /**< statistics poll interval [s] */
    int timeout_ms;     /**< per-request timeout [ms] */
    int verify_tls;     /**< verify the peer certificate (https only) */
} agh_config_t;

/**
 * Load the configuration.
 *
 * The password may come from the AGH_DASH_PASSWORD environment variable, which
 * takes precedence: a convenience for trying the dashboard out without storing
 * a secret anywhere.
 *
 * @param path      the TOML file to read
 * @param out       filled on success
 * @param err       buffer for the error message
 * @param err_len   its size
 * @return 0 on success, -1 on error (err describes what is wrong)
 */
int agh_config_load(const char *path, agh_config_t *out, char *err, size_t err_len);

#endif /*AGH_DASH_CONFIG_H*/
