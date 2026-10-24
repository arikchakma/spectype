//! The closed string sets of the `OpenAPI` 3 vocabulary: component kinds, HTTP methods and
//! parameter locations.

use std::fmt;
use std::str::FromStr;

/// A name outside the set it was parsed as, naming that set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("not a known {0}")]
pub struct UnknownName(&'static str);

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident, $what:literal, [$($(#[$vmeta:meta])* $variant:ident = $text:literal),+ $(,)?]) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// The name as the specification spells it.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = UnknownName;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s {
                    $($text => Ok(Self::$variant),)+
                    _ => Err(UnknownName($what)),
                }
            }
        }

        impl TryFrom<&str> for $name {
            type Error = UnknownName;

            fn try_from(s: &str) -> Result<Self, Self::Error> {
                s.parse()
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool {
                self.as_str() == other
            }
        }
    };
}

string_enum!(
    /// A key of the Components Object.
    ComponentKind,
    "component kind",
    [
        /// `schemas`
        Schemas = "schemas",
        /// `responses`
        Responses = "responses",
        /// `parameters`
        Parameters = "parameters",
        /// `examples`
        Examples = "examples",
        /// `requestBodies`
        RequestBodies = "requestBodies",
        /// `headers`
        Headers = "headers",
        /// `securitySchemes`
        SecuritySchemes = "securitySchemes",
        /// `links`
        Links = "links",
        /// `callbacks`
        Callbacks = "callbacks",
        /// `pathItems`
        PathItems = "pathItems",
    ]
);

impl ComponentKind {
    /// The kinds openapi-typescript turns into `components` members, in its order.
    pub const TRANSFORMED: [Self; 6] =
        [Self::Schemas, Self::Responses, Self::Parameters, Self::RequestBodies, Self::Headers, Self::PathItems];
}

string_enum!(
    /// An operation key of the Path Item Object.
    HttpMethod,
    "HTTP method",
    [
        /// `get`
        Get = "get",
        /// `put`
        Put = "put",
        /// `post`
        Post = "post",
        /// `delete`
        Delete = "delete",
        /// `options`
        Options = "options",
        /// `head`
        Head = "head",
        /// `patch`
        Patch = "patch",
        /// `trace`
        Trace = "trace",
    ]
);

impl HttpMethod {
    /// Every method, in the order openapi-typescript visits them.
    pub const ALL: [Self; 8] =
        [Self::Get, Self::Put, Self::Post, Self::Delete, Self::Options, Self::Head, Self::Patch, Self::Trace];
}

string_enum!(
    /// A Parameter Object's `in`.
    ParamLocation,
    "parameter location",
    [
        /// `query`
        Query = "query",
        /// `header`
        Header = "header",
        /// `path`
        Path = "path",
        /// `cookie`
        Cookie = "cookie",
    ]
);

impl ParamLocation {
    /// Every location, in the order openapi-typescript emits them.
    pub const ALL: [Self; 4] = [Self::Query, Self::Header, Self::Path, Self::Cookie];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for m in HttpMethod::ALL {
            assert_eq!(m.as_str().parse::<HttpMethod>(), Ok(m));
        }
        assert_eq!(ComponentKind::try_from("requestBodies"), Ok(ComponentKind::RequestBodies));
        assert_eq!("GET".parse::<HttpMethod>().unwrap_err().to_string(), "not a known HTTP method");
        assert_eq!(ParamLocation::Cookie.to_string(), "cookie");
    }
}
