//! A dependency-free flag-set newtype generator for 7-bit bitmaps.

/// Defines a `u8` bitmap newtype with named flags. Reserved bits are kept.
macro_rules! flags {
    (
        $(#[$meta:meta])*
        pub struct $name:ident {
            $( $(#[$fmeta:meta])* const $flag:ident = $bit:expr; )*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name(u8);

        impl $name {
            /// No flags.
            pub const NONE: Self = Self(0);
            $( $(#[$fmeta])* pub const $flag: Self = Self($bit); )*

            /// From the wire byte (bit 7 is dropped).
            #[must_use]
            pub const fn from_bits(bits: u8) -> Self {
                Self(bits & 0x7F)
            }

            /// The wire byte.
            #[must_use]
            pub const fn bits(self) -> u8 {
                self.0
            }

            /// Whether every flag in `other` is set.
            #[must_use]
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// Both sets of flags.
            #[must_use]
            pub const fn union(self, other: Self) -> Self {
                Self(self.0 | other.0)
            }

            /// Flags set in both.
            #[must_use]
            pub const fn intersection(self, other: Self) -> Self {
                Self(self.0 & other.0)
            }
        }

        impl core::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self {
                self.union(rhs)
            }
        }

        impl core::ops::BitAnd for $name {
            type Output = Self;
            fn bitand(self, rhs: Self) -> Self {
                self.intersection(rhs)
            }
        }

        impl core::fmt::Debug for $name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                let mut set = f.debug_set();
                let mut known = 0u8;
                $(
                    known |= $bit;
                    if self.contains(Self::$flag) {
                        set.entry(&format_args!(stringify!($flag)));
                    }
                )*
                let rest = self.0 & !known;
                if rest != 0 {
                    set.entry(&format_args!("0x{rest:02X}"));
                }
                set.finish()
            }
        }
    };
}

pub(crate) use flags;
