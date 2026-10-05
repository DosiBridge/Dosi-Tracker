import Foundation

/// JSON coders configured for the backend's wire format: camelCase keys (the
/// models spell them out), dates as ``WireDate`` text, and no `\/` escaping
/// (base64 captures are mostly slashes).
///
/// Each call returns a new coder, so they are never shared between threads.
public enum WireJSON {
    public static func encoder() -> JSONEncoder {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        encoder.dateEncodingStrategy = .custom { date, dateEncoder in
            var container = dateEncoder.singleValueContainer()
            try container.encode(WireDate.string(from: date))
        }
        return encoder
    }

    public static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .custom { dateDecoder in
            let container = try dateDecoder.singleValueContainer()
            let text = try container.decode(String.self)
            guard let date = WireDate.date(from: text) else {
                throw DecodingError.dataCorruptedError(
                    in: container, debugDescription: "Unreadable date: \(text)")
            }
            return date
        }
        return decoder
    }
}
