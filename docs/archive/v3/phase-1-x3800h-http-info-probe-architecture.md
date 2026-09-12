# Version 3, Phase 1 architecture — HTTP information probe

The probe remains presentation-only. The protocol crate owns read-only
AppCommand request construction and XML parsing; infrastructure owns bounded
HTTP transport. The diagnostics binary plans retries and renders evidence.

```text
fixed five-command batch (0300)
             |
      HTTP + AppCommand parse
             |
   returned requested commands?
      |                 |
    none              retain result
      |                 |
0301 compatibility   retry only missing commands,
observation, then    individually at 0300
individual 0300            |
      \___________________/
                |
       any usable response -> exit 0
```

“Usable” is deliberately narrower than “an HTTP request completed”: it needs a
2xx response, valid AppCommand XML, and at least one requested command. This
prevents an unrelated, malformed, or empty reply from being reported as a
successful diagnostic while preserving all of its raw evidence.

The request writer keeps line breaks between structural XML elements. On the
validated AVC-X3800H firmware, a compact XML serialization produced an empty
response even though the endpoint, HTTP status, request names, and parameters
were otherwise identical. Empty parameter elements remain on one line to
preserve the accepted request shape.

Unknown commands and fields are never interpreted by the retry planner. The
planner compares only the five fixed requested command names, so unknown XML
still appears in evidence but cannot promote receiver capabilities.
