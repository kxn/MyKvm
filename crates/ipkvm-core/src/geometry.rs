use crate::{InputError, InputResult};

pub fn map_framebuffer_axis(coordinate: u32, extent: u32) -> InputResult<u16> {
    if extent == 0 {
        return Err(InputError::InvalidFramebufferSize {
            width: 0,
            height: 0,
        });
    }
    if coordinate >= extent {
        return Err(InputError::PointerOutOfBounds { coordinate, extent });
    }

    Ok(((4096 * u64::from(coordinate)) / u64::from(extent)) as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputError;
    use proptest::prelude::*;

    #[test]
    fn pointer_mapping_matches_vendor_formula() {
        assert_eq!(map_framebuffer_axis(100, 1280).unwrap(), 320);
        assert_eq!(map_framebuffer_axis(1279, 1280).unwrap(), 4092);
    }

    #[test]
    fn pointer_mapping_rejects_invalid_coordinate() {
        assert_eq!(
            map_framebuffer_axis(1280, 1280),
            Err(InputError::PointerOutOfBounds {
                coordinate: 1280,
                extent: 1280,
            })
        );
        assert_eq!(
            map_framebuffer_axis(0, 0),
            Err(InputError::InvalidFramebufferSize {
                width: 0,
                height: 0,
            })
        );
    }

    proptest! {
        #[test]
        fn mapped_coordinates_stay_in_range(coordinate in any::<u16>(), extent in 1u16..) {
            let coordinate = u32::from(coordinate) % u32::from(extent);
            prop_assert!(
                map_framebuffer_axis(coordinate, u32::from(extent)).unwrap() <= 4095
            );
        }
    }
}
