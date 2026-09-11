<?php
/**
 * greenpng results SDK (PHP) — backend only.
 *
 * Thin, read-only client: getResult / waitForResult / query. It does NOT
 * collect or relay browser probes — it only reads stored analysis results
 * per session.
 *
 * Auth: X-Gr-Sdk-Key with the site-scoped backend key.
 * Projection: "public" | "sdk" | "diagnostic" (diagnostic needs an ops key).
 */

final class GrApiError extends Exception
{
    /** @var int|null */
    public $status;
    /** @var mixed */
    public $body;

    /** @param mixed $body */
    public function __construct(string $message, ?int $status = null, $body = null)
    {
        parent::__construct($message);
        $this->status = $status;
        $this->body = $body;
    }
}

final class GrAnalysisPending extends Exception
{
    /** @var mixed */
    public $lastBody;

    /** @param mixed $lastBody */
    public function __construct($lastBody = null)
    {
        parent::__construct('analysis_pending');
        $this->lastBody = $lastBody;
    }
}

final class GrResultClient
{
    /** @var string */
    private $baseUrl;
    /** @var string */
    private $apiKey;
    /** @var int */
    private $timeoutMs;

    public function __construct(string $baseUrl, string $apiKey, int $timeoutMs = 15000)
    {
        $this->baseUrl = rtrim($baseUrl, '/');
        $this->apiKey = $apiKey;
        $this->timeoutMs = $timeoutMs;
    }

    /**
     * @return array<string,mixed>
     */
    public function getResult(string $sessionId, string $projection = 'sdk', array $options = []): array
    {
        $url = $this->baseUrl . '/v1/session/' . rawurlencode($sessionId)
            . '/result?' . http_build_query(array_filter(array_merge(
                ['projection' => $projection],
                [
                    'strategy_id' => $options['strategy_id'] ?? null,
                    'response_profile' => $options['response_profile'] ?? null,
                    'profile_cap' => $options['profile_cap'] ?? null,
                    'lang' => $options['lang'] ?? null,
                ],
            ), static function ($value) {
                return $value !== null && $value !== '';
            }));
        return $this->fetch($url);
    }

    /**
     * Poll until the analysis is no longer pending or timeoutMs elapses.
     *
     * @return array<string,mixed>
     */
    public function waitForResult(string $sessionId, string $projection = 'sdk', int $timeoutMs = 8000, int $intervalMs = 500, array $options = []): array
    {
        $deadline = microtime(true) + $timeoutMs / 1000.0;
        $last = null;
        while (true) {
            $last = $this->getResult($sessionId, $projection, $options);
            if (isset($last['ok']) && $last['ok'] === true) {
                $hasBody = isset($last['product_public']) || isset($last['sdk_projection']);
                $pending = false;
                if (isset($last['product_public']['meta']['analysis_pending'])) {
                    $pending = (bool) $last['product_public']['meta']['analysis_pending'];
                }
                if ($hasBody && !$pending) {
                    return $last;
                }
            }
            if (microtime(true) >= $deadline) {
                throw new GrAnalysisPending($last);
            }
            usleep($intervalMs * 1000);
        }
    }

    /**
     * Query helper: wait=false is a single fetch; wait=true polls.
     *
     * @return array<string,mixed>
     */
    public function query(string $sessionId, string $projection = 'sdk', bool $wait = false, int $timeoutMs = 8000, int $intervalMs = 500, array $options = []): array
    {
        if ($wait) {
            return $this->waitForResult($sessionId, $projection, $timeoutMs, $intervalMs, $options);
        }
        return $this->getResult($sessionId, $projection, $options);
    }

    /**
     * @return array<string,mixed>
     */
    private function fetch(string $url): array
    {
        $ch = curl_init($url);
        $headers = [
            'Accept: application/json',
            'X-Gr-Sdk-Key: ' . $this->apiKey,
            'X-Request-Id: req_' . (int) (microtime(true) * 1000),
        ];
        curl_setopt_array($ch, [
            CURLOPT_RETURNTRANSFER => true,
            CURLOPT_HTTPHEADER => $headers,
            CURLOPT_TIMEOUT_MS => $this->timeoutMs,
            CURLOPT_FOLLOWLOCATION => false,
        ]);
        $raw = curl_exec($ch);
        $status = (int) curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
        $err = curl_error($ch);
        curl_close($ch);
        if ($raw === false) {
            throw new GrApiError((string) $err, $status);
        }
        $body = json_decode($raw, true);
        if (!is_array($body)) {
            $body = ['error' => 'bad_json_' . $status];
        }
        if ($status >= 400) {
            $msg = $body['error'] ?? null;
            if (is_array($msg)) {
                $msg = $msg['code'] ?? $msg['message'] ?? 'http_' . $status;
            }
            if (!is_string($msg) || strpos($msg, 'http_') === 0) {
                $msg = 'http_' . $status;
            }
            throw new GrApiError($msg, $status, $body);
        }
        return $body;
    }

    /**
     * Extract site-allowlisted cookies captured server-side (business ids).
     *
     * @param array<string,mixed> $result
     * @return array<string,string>|null
     */
    public static function cookieFields(array $result): ?array
    {
        foreach (['sdk_projection', 'product_public'] as $key) {
            if (isset($result[$key]['cookie_fields']) && is_array($result[$key]['cookie_fields'])) {
                $cf = $result[$key]['cookie_fields'];
                return $cf === [] ? null : $cf;
            }
        }
        if (isset($result['cookie_fields']) && is_array($result['cookie_fields'])) {
            $cf = $result['cookie_fields'];
            return $cf === [] ? null : $cf;
        }
        return null;
    }
}
