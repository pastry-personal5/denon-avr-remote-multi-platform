# Phase 7 query fixtures

These are parser/framing fixtures, not X3800H validation records. They use
representative documented command families and deliberately do not claim that
any response exposes live input or output channel maps.

| Query | Representative raw response | Interpretation |
| --- | --- | --- |
| `SI?\r` | `SICD\r` | input selection `CD` |
| `SD?\r` | `SDHDMI\r` | input-mode setting `HDMI` |
| `DC?\r` | `DCAUTO\r` | decoder setting `AUTO` |
| `MS?\r` | `MSSTEREO\r` | current mode `STEREO` |
| `CV?\r` | `CVFL 00\r` | front-left channel trim/configuration |
| `SYSDA ?\r` | `SYSDA PCM\r` | candidate input encoding |
| `OPINFINS ?\r` | `OPINFINS 222200000000\r` | candidate input channel status string |
| `OPINFASP ?\r` | `OPINFASP 222200000000\r` | candidate active output channel status string |
| `SYSMI ?\r` | `SYSMI Stereo\r` | candidate output sound description |
| `SSINFAISFSV ?\r` | `SSINFAISFSV 48K\r` | candidate sample rate |

Unsupported, malformed, and timed-out replies remain raw observations with an
independent unavailable status. The fixtures intentionally contain no codec,
sample-rate, format, input-map, or output-map promotion.

## HTTP information request

The diagnostic HTTP probe posts the following read-only operations to
`/goform/AppCommand0300.xml`:

```xml
<tx>
  <cmd id="3">
    <name>GetAudioInfo</name>
    <list>
      <param name="inputmode"></param>
      <param name="output"></param>
      <param name="signal"></param>
      <param name="sound"></param>
      <param name="fs"></param>
    </list>
  </cmd>
  <cmd id="3">
    <name>GetInputSignal</name>
    <list><param name="inputsigall"></param></list>
  </cmd>
  <cmd id="3">
    <name>GetActiveSpeaker</name>
    <list><param name="activespall"></param></list>
  </cmd>
</tx>
```

The request is generated from typed, read-only query objects rather than a raw
XML constant. Dynamic identifiers are validated and XML-escaped. Responses are
grouped by `<cmd>`, retained verbatim, and preserve every `<param>` attribute as
well as padded text. Convenience accessors expose trimmed text and numeric
`control` codes without assigning cross-model meaning.

The live X3800H fixture includes an HTTP/1.0 response, padded `PCM` signal text,
an unknown future attribute, empty parameters, and control codes `0`, `1`, and
`2`. Malformed roots, nested commands, missing names, orphan parameters,
duplicate attributes, truncated bodies, invalid HTTP status lines, duplicate
content lengths, and chunked transfer encoding are rejected explicitly.
