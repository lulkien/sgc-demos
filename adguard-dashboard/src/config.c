/**
 * @file config.c
 * TOML configuration loading (tomlc99) with validation.
 */

#include "config.h"

#include <errno.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>

#include "toml.h"

#define DEFAULT_BASE_URL     "http://127.0.0.1:8080"
#define DEFAULT_REFRESH_SECS 5
#define DEFAULT_TIMEOUT_MS   10000

static void set_err(char * err, size_t err_len, const char * fmt, ...)
{
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(err, err_len, fmt, ap);
    va_end(ap);
}

/** Read a string key; dst keeps its previous value (the default) when absent. */
static bool cfg_string(toml_table_t * t, const char * key, char * dst, size_t cap, char * err, size_t err_len)
{
    if(!toml_key_exists(t, key)) return true;

    toml_datum_t v = toml_string_in(t, key);
    if(!v.ok) {
        set_err(err, err_len, "'%s' must be a string", key);
        return false;
    }
    if(strlen(v.u.s) >= cap) {
        set_err(err, err_len, "'%s' is too long (%zu characters max)", key, cap - 1);
        free(v.u.s);
        return false;
    }
    strcpy(dst, v.u.s);
    free(v.u.s);
    return true;
}

/** Read an integer key; dst keeps its previous value (the default) when absent. */
static bool cfg_int(toml_table_t * t, const char * key, int * dst, char * err, size_t err_len)
{
    if(!toml_key_exists(t, key)) return true;

    toml_datum_t v = toml_int_in(t, key);
    if(!v.ok) {
        set_err(err, err_len, "'%s' must be an integer", key);
        return false;
    }
    *dst = (int)v.u.i;
    return true;
}

/** Read a boolean key; dst keeps its previous value (the default) when absent. */
static bool cfg_bool(toml_table_t * t, const char * key, int * dst, char * err, size_t err_len)
{
    if(!toml_key_exists(t, key)) return true;

    toml_datum_t v = toml_bool_in(t, key);
    if(!v.ok) {
        set_err(err, err_len, "'%s' must be a boolean", key);
        return false;
    }
    *dst = v.u.b ? 1 : 0;
    return true;
}

int agh_config_load(const char * path, agh_config_t * out, char * err, size_t err_len)
{
    memset(out, 0, sizeof(*out));
    snprintf(out->base_url, sizeof(out->base_url), "%s", DEFAULT_BASE_URL);
    out->refresh_secs = DEFAULT_REFRESH_SECS;
    out->timeout_ms = DEFAULT_TIMEOUT_MS;
    out->verify_tls = 0;

    FILE * fp = fopen(path, "r");
    if(fp == NULL) {
        set_err(err, err_len, "cannot open %s: %s", path, strerror(errno));
        return -1;
    }

    /* The file carries a password: shout if it is loose. */
    struct stat st;
    if(fstat(fileno(fp), &st) == 0 && (st.st_mode & (S_IRGRP | S_IWGRP | S_IROTH | S_IWOTH))) {
        fprintf(stderr, "[agh-dash] warning: %s mode is %04o - it holds credentials, use chmod 600\n",
                path, (unsigned)(st.st_mode & 07777));
    }

    char parse_err[256] = {0};
    toml_table_t * root = toml_parse_file(fp, parse_err, sizeof(parse_err));
    fclose(fp);
    if(root == NULL) {
        set_err(err, err_len, "%s: %s", path, parse_err[0] ? parse_err : "not a TOML file");
        return -1;
    }

    bool ok = cfg_string(root, "base_url", out->base_url, sizeof(out->base_url), err, err_len)
              && cfg_string(root, "username", out->username, sizeof(out->username), err, err_len)
              && cfg_string(root, "password", out->password, sizeof(out->password), err, err_len)
              && cfg_int(root, "refresh_secs", &out->refresh_secs, err, err_len)
              && cfg_int(root, "timeout_ms", &out->timeout_ms, err, err_len)
              && cfg_bool(root, "verify_tls", &out->verify_tls, err, err_len);
    toml_free(root);
    if(!ok) return -1;

    /* Test convenience: the environment wins, so no secret has to be written. */
    const char * env_pw = getenv("AGH_DASH_PASSWORD");
    if(env_pw != NULL && env_pw[0] != '\0') {
        snprintf(out->password, sizeof(out->password), "%s", env_pw);
        fprintf(stderr, "[agh-dash] using the password from AGH_DASH_PASSWORD\n");
    }

    /* Normalise and validate. */
    size_t len = strlen(out->base_url);
    while(len > 0 && out->base_url[len - 1] == '/') out->base_url[--len] = '\0';

    if(out->base_url[0] == '\0') {
        set_err(err, err_len, "base_url is empty");
        return -1;
    }
    if(strncmp(out->base_url, "http://", 7) != 0 && strncmp(out->base_url, "https://", 8) != 0) {
        set_err(err, err_len, "base_url must start with http:// or https:// (got '%s')", out->base_url);
        return -1;
    }
    if(out->username[0] == '\0') {
        set_err(err, err_len, "username is empty");
        return -1;
    }
    if(out->password[0] == '\0') {
        set_err(err, err_len, "password is empty (set it in %s or in AGH_DASH_PASSWORD)", path);
        return -1;
    }
    if(out->refresh_secs < 1 || out->refresh_secs > 3600) {
        set_err(err, err_len, "refresh_secs must be between 1 and 3600 (got %d)", out->refresh_secs);
        return -1;
    }
    if(out->timeout_ms < 100 || out->timeout_ms > 60000) {
        set_err(err, err_len, "timeout_ms must be between 100 and 60000 (got %d)", out->timeout_ms);
        return -1;
    }

    return 0;
}
