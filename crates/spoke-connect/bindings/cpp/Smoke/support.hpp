/*
 * Shared support for the C++17 smoke translation units.
 *
 * `Smoke/main.cpp` reads the shared golden vector once, parses it here and
 * hands the parsed value to the other units, so no translation unit embeds a
 * copy of a golden constant and there is one JSON reader, not two. Including
 * `spoke_connect.hpp` from this header is also what puts the convenience layer
 * in both linked translation units, so the smoke build covers single-header
 * ODR rather than only compiling the header once.
 */

#ifndef SPOKE_CONNECT_SMOKE_SUPPORT_HPP
#define SPOKE_CONNECT_SMOKE_SUPPORT_HPP

#include "spoke_connect.hpp"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

namespace spoke_smoke {

/** The label the shared fixture read and parse report their failures against. */
inline constexpr const char* kFixtureLabel = "golden fixture";

[[noreturn]] inline void fail(const char* label, const std::string& detail) {
    std::fprintf(stderr, "%s: FAIL\n  %s\n", label, detail.c_str());
    std::fflush(stderr);
    std::exit(1);
}

inline void check(bool condition, const char* label, const std::string& detail) {
    if (!condition) fail(label, detail);
}

/** Prints the group's banner once every assertion in the group has passed. */
inline void banner(const char* label) {
    std::printf("%s: PASS\n", label);
    std::fflush(stdout);
}

/** The shared `golden-hello.json` fields the smoke consumes. */
struct Golden {
    std::vector<uint8_t> seed;
    std::vector<uint8_t> pubkey;
    std::string peer_id;
    std::string nonce;
    std::string manifest_json;
    std::string signature_b64u;
};

inline std::string read_file(const std::string& path) {
    std::ifstream stream(path.c_str(), std::ios::in | std::ios::binary);
    if (!stream) fail(kFixtureLabel, "cannot read the golden vector at " + path);
    const std::string text((std::istreambuf_iterator<char>(stream)),
                           std::istreambuf_iterator<char>());
    if (text.empty()) fail(kFixtureLabel, "the golden vector at " + path + " is empty");
    return text;
}

inline int hex_nibble(char digit) {
    if (digit >= '0' && digit <= '9') return digit - '0';
    if (digit >= 'a' && digit <= 'f') return digit - 'a' + 10;
    if (digit >= 'A' && digit <= 'F') return digit - 'A' + 10;
    fail(kFixtureLabel, std::string("invalid hex digit '") + digit + "'");
}

inline std::vector<uint8_t> hex_bytes(const std::string& hex, const std::string& key) {
    if (hex.empty() || hex.size() % 2 != 0) {
        fail(kFixtureLabel, "\"" + key + "\" is not an even-length hex string");
    }
    std::vector<uint8_t> bytes;
    bytes.reserve(hex.size() / 2);
    for (size_t index = 0; index < hex.size(); index += 2) {
        const int high = hex_nibble(hex[index]);
        const int low = hex_nibble(hex[index + 1]);
        bytes.push_back(static_cast<uint8_t>((high << 4) | low));
    }
    return bytes;
}

/**
 * Reads one string field out of a JSON document, decoding the escapes the
 * fixture uses. The smoke consumes a handful of named fields from a document it
 * also gets to control, so a full JSON parser would be weight without benefit;
 * an unexpected shape fails the run instead of being skipped.
 */
inline std::string json_string_field(const std::string& json, const std::string& key) {
    const std::string needle = "\"" + key + "\"";
    const size_t key_at = json.find(needle);
    if (key_at == std::string::npos) {
        fail(kFixtureLabel, "the document has no \"" + key + "\" field");
    }
    size_t at = json.find(':', key_at + needle.size());
    if (at == std::string::npos) {
        fail(kFixtureLabel, "\"" + key + "\" has no value");
    }
    at += 1;
    while (at < json.size() &&
           (json[at] == ' ' || json[at] == '\n' || json[at] == '\r' || json[at] == '\t')) {
        at += 1;
    }
    if (at >= json.size() || json[at] != '"') {
        fail(kFixtureLabel, "\"" + key + "\" is not a string field");
    }
    std::string value;
    for (size_t index = at + 1; index < json.size(); index += 1) {
        const char current = json[index];
        if (current == '"') return value;
        if (current != '\\') {
            value.push_back(current);
            continue;
        }
        index += 1;
        if (index >= json.size()) break;
        switch (json[index]) {
            case '"': value.push_back('"'); break;
            case '\\': value.push_back('\\'); break;
            case '/': value.push_back('/'); break;
            case 'b': value.push_back('\b'); break;
            case 'f': value.push_back('\f'); break;
            case 'n': value.push_back('\n'); break;
            case 'r': value.push_back('\r'); break;
            case 't': value.push_back('\t'); break;
            default:
                fail(kFixtureLabel, std::string("unsupported escape in \"") + key + "\"");
        }
    }
    fail(kFixtureLabel, "\"" + key + "\" has an unterminated string value");
}

inline Golden load_golden(const std::string& fixture) {
    Golden golden;
    golden.seed = hex_bytes(json_string_field(fixture, "seed_hex"), "seed_hex");
    golden.pubkey = hex_bytes(json_string_field(fixture, "pubkey_hex"), "pubkey_hex");
    golden.peer_id = json_string_field(fixture, "peer_id");
    golden.nonce = json_string_field(fixture, "nonce");
    golden.manifest_json = json_string_field(fixture, "manifest_json");
    golden.signature_b64u = json_string_field(fixture, "signature_b64u");
    check(golden.seed.size() == 32, kFixtureLabel, "the golden seed is not 32 bytes");
    check(golden.pubkey.size() == 32, kFixtureLabel, "the golden public key is not 32 bytes");
    return golden;
}

/**
 * The convenience-layer proofs in `Smoke/convenience.cpp`. `main.cpp` calls
 * each group once with the golden vector it read, so the second translation
 * unit never re-reads or re-transcribes the fixture: the value/ownership group
 * first, then the callback bridges with the adapter and responder wrappers.
 */
void run_convenience_values(const Golden& golden);
void run_convenience_session(const Golden& golden);

}  // namespace spoke_smoke

#endif /* SPOKE_CONNECT_SMOKE_SUPPORT_HPP */
