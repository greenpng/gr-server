// Package grresults is a backend-only, read-only client for greenpng
// analysis results. It does NOT collect or relay browser probes — probes
// keep flowing straight to the probe server; this client only queries
// the stored result per session.
//
// Surface: GetResult / WaitForResult / Query + CookieFields helper.
// Auth: X-Gr-Sdk-Key with the site-scoped backend key.
// Projection: "public" | "sdk" | "diagnostic" (diagnostic needs an ops key).
package grresults

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

// AnalysisPending is returned by WaitForResult when the session analysis is
// still pending when the timeout elapses.
type AnalysisPending struct {
	LastBody json.RawMessage
}

func (e *AnalysisPending) Error() string { return "analysis_pending" }

// APIError carries the HTTP status and parsed error body.
type APIError struct {
	Status int
	Body   map[string]interface{}
}

func (e *APIError) Error() string {
	if e.Body != nil {
		if err, ok := e.Body["error"].(string); ok && !strings.HasPrefix(err, "http_") {
			return err
		}
		if errObj, ok := e.Body["error"].(map[string]interface{}); ok {
			if code, ok := errObj["code"].(string); ok {
				return code
			}
		}
	}
	if e.Status > 0 {
		return fmt.Sprintf("http_%d", e.Status)
	}
	return "api_error"
}

// Client is a thin result client for one probe server + one site key.
type Client struct {
	BaseURL string
	APIKey  string
	HTTP    *http.Client

	expectedSchema string
}

// ResultOptions are optional v2 result projection controls. Empty fields use
// the site's configured defaults.
type ResultOptions struct {
	StrategyID      string
	ResponseProfile string
	ProfileCap      string
	Lang            string
}

// New creates a result client. baseURL e.g. "https://probe.example.com".
func New(baseURL, apiKey string) *Client {
	return &Client{
		BaseURL:        strings.TrimRight(baseURL, "/"),
		APIKey:         apiKey,
		HTTP:           &http.Client{Timeout: 15 * time.Second},
		expectedSchema: "product_public_v1",
	}
}

func (c *Client) resultURL(sessionID, projection string, opts ResultOptions) string {
	q := url.Values{}
	q.Set("projection", projection)
	if opts.StrategyID != "" {
		q.Set("strategy_id", opts.StrategyID)
	}
	if opts.ResponseProfile != "" {
		q.Set("response_profile", opts.ResponseProfile)
	}
	if opts.ProfileCap != "" {
		q.Set("profile_cap", opts.ProfileCap)
	}
	if opts.Lang != "" {
		q.Set("lang", opts.Lang)
	}
	return fmt.Sprintf("%s/v1/session/%s/result?%s",
		c.BaseURL, url.PathEscape(sessionID), q.Encode())
}

func (c *Client) fetch(rawURL string) (map[string]interface{}, error) {
	req, err := http.NewRequest(http.MethodGet, rawURL, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Accept", "application/json")
	req.Header.Set("X-Gr-Sdk-Key", c.APIKey)
	req.Header.Set("X-Request-Id", fmt.Sprintf("req_%d", time.Now().UnixMilli()))
	resp, err := c.HTTP.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	body := map[string]interface{}{}
	if err := json.Unmarshal(raw, &body); err != nil {
		body = map[string]interface{}{"error": fmt.Sprintf("bad_json_%d", resp.StatusCode)}
	}
	if resp.StatusCode >= 400 {
		return body, &APIError{Status: resp.StatusCode, Body: body}
	}
	return body, nil
}

// GetResult fetches one result snapshot for a session.
func (c *Client) GetResult(sessionID, projection string, options ...ResultOptions) (map[string]interface{}, error) {
	var opts ResultOptions
	if len(options) > 0 {
		opts = options[0]
	}
	return c.fetch(c.resultURL(sessionID, projection, opts))
}

// GetResultWithOptions is the explicit v2 form of GetResult.
func (c *Client) GetResultWithOptions(sessionID, projection string, opts ResultOptions) (map[string]interface{}, error) {
	return c.GetResult(sessionID, projection, opts)
}

// WaitForResult polls until the analysis is no longer pending or the timeout
// elapses (AnalysisPending).
func (c *Client) WaitForResult(sessionID, projection string, timeoutMs, intervalMs int, options ...ResultOptions) (map[string]interface{}, error) {
	if timeoutMs <= 0 {
		timeoutMs = 8000
	}
	if intervalMs <= 0 {
		intervalMs = 500
	}
	deadline := time.Now().Add(time.Duration(timeoutMs) * time.Millisecond)
	var last map[string]interface{}
	for {
		last, _ = c.GetResult(sessionID, projection, options...)
		if ok, _ := last["ok"].(bool); ok {
			hasBody := false
			if _, yes := last["product_public"]; yes {
				hasBody = true
			}
			if _, yes := last["sdk_projection"]; yes {
				hasBody = true
			}
			pending := false
			if pp, yes := last["product_public"].(map[string]interface{}); yes {
				if meta, ok := pp["meta"].(map[string]interface{}); ok {
					pending, _ = meta["analysis_pending"].(bool)
				}
			}
			if hasBody && !pending {
				return last, nil
			}
		}
		if time.Now().After(deadline) {
			raw, _ := json.Marshal(last)
			return nil, &AnalysisPending{LastBody: raw}
		}
		time.Sleep(time.Duration(intervalMs) * time.Millisecond)
	}
}

// Query is an alias-style helper: with wait=false it is a single GetResult;
// with wait=true it polls like WaitForResult.
func (c *Client) Query(sessionID, projection string, wait bool, timeoutMs, intervalMs int, options ...ResultOptions) (map[string]interface{}, error) {
	if wait {
		return c.WaitForResult(sessionID, projection, timeoutMs, intervalMs, options...)
	}
	return c.GetResult(sessionID, projection, options...)
}

// CookieFields extracts the site-owner allowlisted cookies captured
// server-side at session open/ingest (business identifiers). Returns nil
// when the site has no cookie allowlist or nothing was captured.
func CookieFields(result map[string]interface{}) (map[string]interface{}, error) {
	look := func(node map[string]interface{}) (map[string]interface{}, bool) {
		cf, ok := node["cookie_fields"].(map[string]interface{})
		if !ok || len(cf) == 0 {
			return nil, false
		}
		return cf, true
	}
	if sp, ok := result["sdk_projection"].(map[string]interface{}); ok {
		if cf, ok := look(sp); ok {
			return cf, nil
		}
	}
	if pp, ok := result["product_public"].(map[string]interface{}); ok {
		if cf, ok := look(pp); ok {
			return cf, nil
		}
	}
	if cf, ok := look(result); ok {
		return cf, nil
	}
	return nil, errors.New("no cookie_fields in result")
}
