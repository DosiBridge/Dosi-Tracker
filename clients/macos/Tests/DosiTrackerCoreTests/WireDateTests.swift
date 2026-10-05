import XCTest
@testable import DosiTrackerCore

/// Date text as the backend reads and writes it, integer Unix time, and the
/// percent-encoding shared with the Windows agent.
final class WireDateTests: XCTestCase {
    func testDatesAreWrittenInUTCWithMilliseconds() {
        XCTAssertEqual(WireDate.string(from: Date(timeIntervalSince1970: 0)), "1970-01-01T00:00:00.000Z")
        XCTAssertEqual(WireDate.string(from: utc("2026-07-14T10:05:00.5Z")), "2026-07-14T10:05:00.500Z")
        XCTAssertEqual(WireDate.string(from: Date(timeIntervalSince1970: 1_790_000_000.0004)), "2026-09-21T14:13:20.000Z")
        XCTAssertEqual(WireDate.string(from: Date(timeIntervalSince1970: -1.5)), "1969-12-31T23:59:58.500Z")
        XCTAssertEqual(WireDate.string(from: utc("2028-02-29T23:59:59.999Z")), "2028-02-29T23:59:59.999Z")
    }

    func testEveryShapeTheServerSendsIsRead() {
        let expected = utc("2026-07-14T10:05:00Z")
        XCTAssertEqual(WireDate.date(from: "2026-07-14T10:05:00Z"), expected)
        XCTAssertEqual(WireDate.date(from: "2026-07-14T10:05:00.0000000Z"), expected)
        XCTAssertEqual(WireDate.date(from: "2026-07-14T16:05:00+06:00"), expected)
        XCTAssertEqual(WireDate.date(from: "2026-07-14T05:05:00-0500"), expected)
        XCTAssertEqual(WireDate.date(from: "2026-07-14T10:05:00"), expected, "no zone: UTC (Kind Unspecified)")
        XCTAssertEqual(WireDate.date(from: "2026-07-14 10:05"), expected)
        XCTAssertEqual(WireDate.date(from: " 2026-07-14t10:05:00z "), expected)
        XCTAssertEqual(WireDate.date(from: "2026-07-14T10:05:00.1234567Z").map { EpochTime.milliseconds($0) },
                       EpochTime.milliseconds(expected) + 123)
        for bad in ["", "soon", "2026-13-01T00:00:00Z", "2026-02-30T00:00:00Z", "2026-07-14T25:00:00Z",
                    "2026-07-14T10:05:00+19:00", "2026-07-14T10:05:00Zjunk", "2026-07-14"] {
            XCTAssertNil(WireDate.date(from: bad), bad)
        }
    }

    func testHTTPDatesAreRead() {
        XCTAssertEqual(WireDate.httpDate(from: "Mon, 05 Oct 2026 10:03:30 GMT"), utc("2026-10-05T10:03:30Z"))
        XCTAssertEqual(WireDate.httpDate(from: "05 Oct 2026 10:03:30 GMT"), utc("2026-10-05T10:03:30Z"))
        XCTAssertNil(WireDate.httpDate(from: "Mon, 05 Oct 2026 10:03:30 PST"))
        XCTAssertNil(WireDate.httpDate(from: "yesterday"))
        XCTAssertNil(WireDate.httpDate(from: "Mon, 31 Feb 2026 10:03:30 GMT"))
    }

    func testCompactStampsNameFiles() {
        XCTAssertEqual(WireDate.compactStamp(from: utc("2026-10-05T10:00:00Z")), "20261005T100000Z")
        XCTAssertEqual(WireDate.compactStamp(from: utc("1999-12-31T23:59:59.900Z")), "19991231T235959Z")
    }

    func testCalendarArithmeticRoundTripsOverFourCenturies() {
        var day: Int64 = -146_097
        while day <= 146_097 {
            let civil = WireDate.civilFromDays(day)
            XCTAssertEqual(WireDate.daysFromCivil(year: civil.year, month: civil.month, day: civil.day), day)
            day += 997
        }
        XCTAssertEqual(WireDate.daysFromCivil(year: 1970, month: 1, day: 1), 0)
        XCTAssertEqual(WireDate.daysFromCivil(year: 2000, month: 3, day: 1), 11_017)
    }

    func testEpochTimeFloorsTowardNegativeInfinity() {
        XCTAssertEqual(EpochTime.floorDiv(7, 1000), 0)
        XCTAssertEqual(EpochTime.floorDiv(-1, 1000), -1)
        XCTAssertEqual(EpochTime.floorDiv(-1000, 1000), -1)
        XCTAssertEqual(EpochTime.floorDiv(-1001, 1000), -2)
        XCTAssertEqual(EpochTime.seconds(Date(timeIntervalSince1970: -0.5)), -1)
        XCTAssertEqual(EpochTime.seconds(at(t0)), t0)
        XCTAssertEqual(EpochTime.milliseconds(atMs(t0Ms + 7)), t0Ms + 7)
        XCTAssertEqual(EpochTime.date(seconds: t0), at(t0))
    }

    func testPercentEncodingMatchesTheWindowsAgent() {
        XCTAssertEqual(PercentEncoding.encode("Acme Corp/EU"), "Acme%20Corp%2FEU")
        XCTAssertEqual(PercentEncoding.encode("plain-name_1.x~"), "plain-name_1.x~")
        XCTAssertEqual(PercentEncoding.encode("a+b=c&d"), "a%2Bb%3Dc%26d")
        XCTAssertEqual(PercentEncoding.encode("Zürich"), "Z%C3%BCrich")
        XCTAssertEqual(String(decoding: PercentEncoding.form([("grant_type", "password"), ("scope", "Tracker offline_access")]),
                              as: UTF8.self),
                       "grant_type=password&scope=Tracker%20offline_access")
        XCTAssertEqual(PercentEncoding.query([("From", "2026-10-05T10:00:00.000Z")]), "From=2026-10-05T10%3A00%3A00.000Z")
    }

    func testTextIsCutOnCharacterBoundariesInUTF16Units() {
        XCTAssertEqual("abcdef".truncatedUTF16(4), "abcd")
        XCTAssertEqual("abc".truncatedUTF16(10), "abc")
        XCTAssertEqual("a😀b".truncatedUTF16(2), "a", "an emoji takes two units and is never split")
        XCTAssertEqual("a😀b".truncatedUTF16(3), "a😀")
    }
}
