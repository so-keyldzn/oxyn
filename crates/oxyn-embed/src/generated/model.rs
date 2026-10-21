// Generated from ONNX "onnx/model.onnx" by burn-onnx
extern crate alloc;
use burn::nn::LayerNorm;
use burn::nn::LayerNormConfig;
use burn::nn::Linear;
use burn::nn::LinearConfig;
use burn::prelude::*;
use burn::tensor::TensorData;

#[derive(Module, Debug)]
pub struct Submodule1 {
    constant88: burn::module::Param<Tensor<1, Int>>,
    constant89: burn::module::Param<Tensor<1>>,
    constant1: burn::module::Param<Tensor<2>>,
    layernormalization1: LayerNorm,
    linear1: Linear,
    constant112: burn::module::Param<Tensor<1, Int>>,
    constant106: burn::module::Param<Tensor<3>>,
    constant144: burn::module::Param<Tensor<1>>,
    linear2: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule1 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let constant88: burn::module::Param<Tensor<1, Int>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1, Int>::from_data(
                    burn::tensor::TensorData::from([-1i64]),
                    (device, burn::tensor::DType::I64),
                )
            },
            device.clone(),
            false,
            [1].into(),
        );
        let constant89: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::from_data(
                    burn::tensor::TensorData::from([1f64]),
                    (device, burn::tensor::DType::F32),
                )
            },
            device.clone(),
            false,
            [1].into(),
        );
        let constant1: burn::module::Param<Tensor<2>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<2>::zeros([180000, 384], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [180000, 384].into(),
        );
        let layernormalization1 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear1 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant112: burn::module::Param<Tensor<1, Int>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1, Int>::from_data(
                    burn::tensor::TensorData::from([-1i64]),
                    (device, burn::tensor::DType::I64),
                )
            },
            device.clone(),
            false,
            [1].into(),
        );
        let constant106: burn::module::Param<Tensor<3>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<3>::zeros([1, 16, 1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1, 16, 1].into(),
        );
        let constant144: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear2 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            constant88,
            constant89,
            constant1,
            layernormalization1,
            linear1,
            constant112,
            constant106,
            constant144,
            linear2,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        input_ids: Tensor<2, Int>,
        attention_mask: Tensor<2, Int>,
    ) -> (
        Tensor<3>,
        i64,
        Tensor<3>,
        Tensor<4>,
        Tensor<4>,
        Tensor<4>,
        Tensor<4>,
    ) {
        let shape1_out1: [i64; 2] = {
            let axes = &input_ids.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather1_out1 = shape1_out1[1] as i64;
        let range1_out1 = {
            let start = 0i64;
            let delta = 1i64;
            assert!(delta != 0);
            let n = (((gather1_out1 as i128 - start as i128) as f64) / delta as f64)
                .ceil()
                .max(0.0) as i64;
            Tensor::arange(0..n, &self.device)
                .cast(burn::tensor::DType::I64)
                .mul_scalar(delta)
                .add_scalar(start)
        };
        let unsqueeze1_out1: Tensor<2, Int> = range1_out1.unsqueeze_dims::<2>(&[0]);
        let shape2_out1: [i64; 2] = {
            let axes = &attention_mask.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather2_out1 = shape2_out1[0] as i64;
        let gather3_out1 = shape2_out1[1] as i64;
        let unsqueeze2_out1: Tensor<3, Int> = attention_mask.unsqueeze_dims::<3>(&[1]);
        let unsqueeze3_out1: Tensor<4, Int> = unsqueeze2_out1.unsqueeze_dims::<4>(&[2]);
        let unsqueeze4_out1 = [gather2_out1 as i64];
        let unsqueeze5_out1 = [gather3_out1 as i64];
        let unsqueeze6_out1 = [gather3_out1 as i64];
        let constant84_out1: [i64; 1] = [1i64];
        let concat1_out1: [i64; 4usize] = [
            &unsqueeze4_out1[..],
            &constant84_out1[..],
            &unsqueeze5_out1[..],
            &unsqueeze6_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape1_out1 = concat1_out1;
        let shape4_out1: [i64; 1] = [4i64];
        let constantofshape1_out1 = Tensor::<1, Int>::from_data(
            burn::tensor::TensorData::from([1i64 as i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .reshape([1])
        .expand(shape4_out1);
        let constant88_out1 = self.constant88.val();
        let mul1_out1 = constantofshape1_out1.clone().mul(constant88_out1);
        let equal1_out1 = Tensor::<1, burn::tensor::Int>::from_data(
            burn::tensor::TensorData::from(&reshape1_out1 as &[i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .equal(mul1_out1);
        let where1_out1 = Tensor::<1, burn::tensor::Int>::from_data(
            burn::tensor::TensorData::from(&reshape1_out1 as &[i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .mask_where(equal1_out1, constantofshape1_out1);
        let expand1_out1 = {
            let onnx_shape: [i64; 4usize] = TryInto::<[i64; 4usize]>::try_into(
                where1_out1.to_data().convert::<i64>().as_slice().unwrap(),
            )
            .unwrap();
            let input_dims = unsqueeze3_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..4usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            unsqueeze3_out1.expand(shape)
        };
        let cast2_out1 = expand1_out1.float().cast(burn::tensor::DType::F32);
        let constant89_out1 = self.constant89.val();
        let sub1_out1 = (constant89_out1)
            .unsqueeze_dims(&[0isize, 1isize, 2isize])
            .sub(cast2_out1);
        let cast3_out1 = sub1_out1.clone().bool();
        let constant90_out1 = -340282350000000000000000000000000000000f32;
        let where2_out1 = sub1_out1.mask_fill(cast3_out1, constant90_out1);
        let shape5_out1: [i64; 4] = {
            let axes = &where2_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather4_out1 = shape5_out1[2] as i64;
        let range2_out1 = {
            let start = 0i64;
            let delta = 1i64;
            assert!(delta != 0);
            let n = (((gather4_out1 as i128 - start as i128) as f64) / delta as f64)
                .ceil()
                .max(0.0) as i64;
            Tensor::arange(0..n, &self.device)
                .cast(burn::tensor::DType::I64)
                .mul_scalar(delta)
                .add_scalar(start)
        };
        let unsqueeze7_out1: Tensor<2, Int> = range2_out1.unsqueeze_dims::<2>(&[0]);
        let transpose1_out1 = unsqueeze7_out1.clone().permute([1, 0]);
        let sub2_out1 = unsqueeze7_out1.sub(transpose1_out1);
        let abs1_out1 = sub2_out1.abs();
        let constant95_out1 = 64i64;
        let lessorequal1_out1 = abs1_out1.lower_equal_elem(constant95_out1);
        let unsqueeze8_out1: Tensor<3, Bool> = lessorequal1_out1.unsqueeze_dims::<3>(&[0]);
        let unsqueeze9_out1: Tensor<4, Bool> = unsqueeze8_out1.unsqueeze_dims::<4>(&[0]);
        let not1_out1 = unsqueeze9_out1.bool_not();
        let constant98_out1 = -340282350000000000000000000000000000000f32;
        let where3_out1 = where2_out1.clone().mask_fill(not1_out1, constant98_out1);
        let constant1_out1 = self.constant1.val();
        let gather5_out1 = {
            let axis_size = constant1_out1.dims()[0] as i64;
            let negative = input_ids.clone().lower_elem(0i64);
            let corrected = input_ids.clone() + axis_size;
            let indices = input_ids.mask_where(negative, corrected);
            constant1_out1.take::<2, 3>(0, indices)
        };
        let layernormalization1_out1 = {
            let dtype = gather5_out1.dtype();
            self.layernormalization1
                .forward(gather5_out1.cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear1_out1 = self.linear1.forward(layernormalization1_out1.clone());
        let shape6_out1: [i64; 3] = {
            let axes = &layernormalization1_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather6_out1 = shape6_out1[0] as i64;
        let unsqueeze10_out1 = [gather6_out1 as i64];
        let constant105_out1: [i64; 1] = [32i64];
        let constant102_out1: [i64; 1] = [-1i64];
        let constant103_out1: [i64; 1] = [3i64];
        let constant104_out1: [i64; 1] = [12i64];
        let concat2_out1: [i64; 5usize] = [
            &unsqueeze10_out1[..],
            &constant102_out1[..],
            &constant103_out1[..],
            &constant104_out1[..],
            &constant105_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape2_out1 = linear1_out1.reshape(concat2_out1);
        let shape7_out1: [i64; 2] = {
            let axes = &unsqueeze1_out1.clone().dims()[0..2];
            let mut output = [0i64; 2];
            for i in 0..2 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather7_out1 = shape7_out1[0] as i64;
        let unsqueeze11_out1 = [gather7_out1 as i64];
        let constant110_out1: [i64; 1] = [1i64];
        let constant109_out1: [i64; 1] = [-1i64];
        let concat3_out1: [i64; 3usize] = [
            &unsqueeze11_out1[..],
            &constant109_out1[..],
            &constant110_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape3_out1 = concat3_out1;
        let shape8_out1: [i64; 1] = [3i64];
        let constantofshape2_out1 = Tensor::<1, Int>::from_data(
            burn::tensor::TensorData::from([1i64 as i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .reshape([1])
        .expand(shape8_out1);
        let constant112_out1 = self.constant112.val();
        let mul2_out1 = constantofshape2_out1.clone().mul(constant112_out1);
        let equal2_out1 = Tensor::<1, burn::tensor::Int>::from_data(
            burn::tensor::TensorData::from(&reshape3_out1 as &[i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .equal(mul2_out1);
        let where4_out1 = Tensor::<1, burn::tensor::Int>::from_data(
            burn::tensor::TensorData::from(&reshape3_out1 as &[i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .mask_where(equal2_out1, constantofshape2_out1);
        let constant106_out1 = self.constant106.val();
        let expand2_out1 = {
            let onnx_shape: [i64; 3usize] = TryInto::<[i64; 3usize]>::try_into(
                where4_out1.to_data().convert::<i64>().as_slice().unwrap(),
            )
            .unwrap();
            let input_dims = constant106_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..3usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            constant106_out1.expand(shape)
        };
        let unsqueeze12_out1: Tensor<3, Int> = unsqueeze1_out1.unsqueeze_dims::<3>(&[1]);
        let cast10_out1 = unsqueeze12_out1.float().cast(burn::tensor::DType::F32);
        let matmul2_out1 = expand2_out1.matmul(cast10_out1.clone());
        let transpose2_out1 = matmul2_out1.permute([0, 2, 1]);
        let concat4_out1 =
            burn::tensor::Tensor::cat([transpose2_out1.clone(), transpose2_out1].into(), 2);
        let cos1_out1 = concat4_out1.clone().cos();
        let sin1_out1 = concat4_out1.sin();
        let transpose3_out1 = reshape2_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose3_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split1_out1, split1_out2, split1_out3] = split_tensors.try_into().unwrap();
        let squeeze1_out1 = split1_out1.squeeze_dims::<4>(&[2]);
        let squeeze2_out1 = split1_out2.squeeze_dims::<4>(&[2]);
        let squeeze3_out1 = split1_out3.squeeze_dims::<4>(&[2]);
        let unsqueeze13_out1: Tensor<4> = cos1_out1.unsqueeze_dims::<4>(&[1]);
        let unsqueeze14_out1: Tensor<4> = sin1_out1.unsqueeze_dims::<4>(&[1]);
        let mul5_out1 = squeeze1_out1.clone().mul(unsqueeze13_out1.clone());
        let shape9_out1: [i64; 4] = {
            let axes = &squeeze1_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather8_out1 = shape9_out1[3] as i64;
        let constant123_out1 = 2i64;
        let div1_out1 = gather8_out1 / constant123_out1;
        let unsqueeze15_out1 = [div1_out1 as i64];
        let slice1_out1 = squeeze1_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze15_out1[0]]);
        let unsqueeze16_out1 = [div1_out1 as i64];
        let slice2_out1 =
            squeeze1_out1.slice(s![.., .., .., unsqueeze16_out1[0]..9223372036854775807]);
        let neg1_out1 = slice2_out1.neg();
        let concat5_out1 = burn::tensor::Tensor::cat([neg1_out1, slice1_out1].into(), 3);
        let mul6_out1 = concat5_out1.mul(unsqueeze14_out1.clone());
        let add1_out1 = mul5_out1.add(mul6_out1);
        let mul7_out1 = squeeze2_out1.clone().mul(unsqueeze13_out1.clone());
        let shape10_out1: [i64; 4] = {
            let axes = &squeeze2_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather9_out1 = shape10_out1[3] as i64;
        let constant133_out1 = 2i64;
        let div2_out1 = gather9_out1 / constant133_out1;
        let unsqueeze17_out1 = [div2_out1 as i64];
        let slice3_out1 = squeeze2_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze17_out1[0]]);
        let unsqueeze18_out1 = [div2_out1 as i64];
        let slice4_out1 =
            squeeze2_out1.slice(s![.., .., .., unsqueeze18_out1[0]..9223372036854775807]);
        let neg2_out1 = slice4_out1.neg();
        let concat6_out1 = burn::tensor::Tensor::cat([neg2_out1, slice3_out1].into(), 3);
        let mul8_out1 = concat6_out1.mul(unsqueeze14_out1.clone());
        let add2_out1 = mul7_out1.add(mul8_out1);
        let shape11_out1: [i64; 4] = {
            let axes = &add1_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice5_out1: [i64; 1] = shape11_out1[3..4].try_into().unwrap();
        let cast19_out1 = {
            let shape_array = slice5_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt1_out1 = cast19_out1.sqrt();
        let constant144_out1 = self.constant144.val();
        let div3_out1 = constant144_out1.div(sqrt1_out1);
        let transpose4_out1 = add2_out1.permute([0, 1, 3, 2]);
        let sqrt2_out1 = div3_out1.sqrt();
        let mul9_out1 =
            add1_out1.mul((sqrt2_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul10_out1 =
            transpose4_out1.mul((sqrt2_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul3_out1 = mul9_out1.matmul(mul10_out1);
        let add3_out1 = matmul3_out1.add(where2_out1.clone());
        let softmax1_out1 = burn::tensor::activation::softmax(add3_out1, 3);
        let matmul4_out1 = softmax1_out1.matmul(squeeze3_out1);
        let transpose5_out1 = matmul4_out1.permute([0, 2, 1, 3]);
        let unsqueeze19_out1 = [gather6_out1 as i64];
        let constant147_out1: [i64; 1] = [384i64];
        let constant146_out1: [i64; 1] = [-1i64];
        let concat7_out1: [i64; 3usize] = [
            &unsqueeze19_out1[..],
            &constant146_out1[..],
            &constant147_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape4_out1 = transpose5_out1.reshape(concat7_out1);
        let linear2_out1 = self.linear2.forward(reshape4_out1);
        let add4_out1 = layernormalization1_out1.add(linear2_out1);
        (
            add4_out1,
            gather7_out1,
            cast10_out1,
            where3_out1,
            unsqueeze13_out1,
            unsqueeze14_out1,
            where2_out1,
        )
    }
}
#[derive(Module, Debug)]
pub struct Submodule2 {
    layernormalization2: LayerNorm,
    linear3: Linear,
    linear4: Linear,
    layernormalization3: LayerNorm,
    linear5: Linear,
    constant167: burn::module::Param<Tensor<1, Int>>,
    constant162: burn::module::Param<Tensor<3>>,
    constant198: burn::module::Param<Tensor<1>>,
    linear6: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule2 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization2 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear3 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear4 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization3 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear5 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant167: burn::module::Param<Tensor<1, Int>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1, Int>::from_data(
                    burn::tensor::TensorData::from([-1i64]),
                    (device, burn::tensor::DType::I64),
                )
            },
            device.clone(),
            false,
            [1].into(),
        );
        let constant162: burn::module::Param<Tensor<3>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<3>::zeros([1, 16, 1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1, 16, 1].into(),
        );
        let constant198: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear6 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization2,
            linear3,
            linear4,
            layernormalization3,
            linear5,
            constant167,
            constant162,
            constant198,
            linear6,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add4_out1: Tensor<3>,
        gather7_out1: i64,
        cast10_out1: Tensor<3>,
        where3_out1: Tensor<4>,
    ) -> (Tensor<3>, Tensor<4>, Tensor<4>) {
        let layernormalization2_out1 = {
            let dtype = add4_out1.clone().dtype();
            self.layernormalization2
                .forward(add4_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear3_out1 = self.linear3.forward(layernormalization2_out1);
        let shape12_out1: [i64; 3] = {
            let axes = &linear3_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant149_out1: [i64; 1] = [-1i64];
        let gather10_out1: [i64; 1usize] = constant149_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape12_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape12_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant151_out1: [i64; 1] = [1i64];
        let add5_out1 = {
            let __lhs = gather10_out1;
            let __rhs = constant151_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant152_out1: [i64; 1] = [2i64];
        let div4_out1 = {
            let __lhs = add5_out1;
            let __rhs = constant152_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice6_out1 = linear3_out1.clone().slice(s![.., .., 0..div4_out1[0]]);
        let constant154_out1: [i64; 1] = [2i64];
        let mul12_out1 = {
            let __lhs = div4_out1;
            let __rhs = constant154_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice7_out1 = linear3_out1.slice(s![.., .., div4_out1[0]..mul12_out1[0]]);
        let sigmoid1_out1 = burn::tensor::activation::sigmoid(slice6_out1.clone());
        let mul13_out1 = slice6_out1.mul(sigmoid1_out1);
        let mul14_out1 = mul13_out1.mul(slice7_out1);
        let linear4_out1 = self.linear4.forward(mul14_out1);
        let add6_out1 = add4_out1.add(linear4_out1);
        let layernormalization3_out1 = {
            let dtype = add6_out1.clone().dtype();
            self.layernormalization3
                .forward(add6_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear5_out1 = self.linear5.forward(layernormalization3_out1.clone());
        let shape13_out1: [i64; 3] = {
            let axes = &layernormalization3_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather11_out1 = shape13_out1[0] as i64;
        let unsqueeze20_out1 = [gather11_out1 as i64];
        let constant161_out1: [i64; 1] = [32i64];
        let constant158_out1: [i64; 1] = [-1i64];
        let constant159_out1: [i64; 1] = [3i64];
        let constant160_out1: [i64; 1] = [12i64];
        let concat8_out1: [i64; 5usize] = [
            &unsqueeze20_out1[..],
            &constant158_out1[..],
            &constant159_out1[..],
            &constant160_out1[..],
            &constant161_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape5_out1 = linear5_out1.reshape(concat8_out1);
        let unsqueeze21_out1 = [gather7_out1 as i64];
        let constant165_out1: [i64; 1] = [1i64];
        let constant164_out1: [i64; 1] = [-1i64];
        let concat9_out1: [i64; 3usize] = [
            &unsqueeze21_out1[..],
            &constant164_out1[..],
            &constant165_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape6_out1 = concat9_out1;
        let shape14_out1: [i64; 1] = [3i64];
        let constantofshape3_out1 = Tensor::<1, Int>::from_data(
            burn::tensor::TensorData::from([1i64 as i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .reshape([1])
        .expand(shape14_out1);
        let constant167_out1 = self.constant167.val();
        let mul15_out1 = constantofshape3_out1.clone().mul(constant167_out1);
        let equal3_out1 = Tensor::<1, burn::tensor::Int>::from_data(
            burn::tensor::TensorData::from(&reshape6_out1 as &[i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .equal(mul15_out1);
        let where5_out1 = Tensor::<1, burn::tensor::Int>::from_data(
            burn::tensor::TensorData::from(&reshape6_out1 as &[i64]),
            (&self.device, burn::tensor::DType::I64),
        )
        .mask_where(equal3_out1, constantofshape3_out1);
        let constant162_out1 = self.constant162.val();
        let expand3_out1 = {
            let onnx_shape: [i64; 3usize] = TryInto::<[i64; 3usize]>::try_into(
                where5_out1.to_data().convert::<i64>().as_slice().unwrap(),
            )
            .unwrap();
            let input_dims = constant162_out1.dims();
            let mut shape = onnx_shape;
            #[allow(clippy::needless_range_loop)]
            for i in 0..3usize {
                let dim_offset = i;
                if shape[dim_offset] == 1 && input_dims[i] > 1 {
                    shape[dim_offset] = input_dims[i] as i64;
                }
            }
            constant162_out1.expand(shape)
        };
        let matmul9_out1 = expand3_out1.matmul(cast10_out1);
        let transpose6_out1 = matmul9_out1.permute([0, 2, 1]);
        let concat10_out1 =
            burn::tensor::Tensor::cat([transpose6_out1.clone(), transpose6_out1].into(), 2);
        let cos2_out1 = concat10_out1.clone().cos();
        let sin2_out1 = concat10_out1.sin();
        let transpose7_out1 = reshape5_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose7_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split2_out1, split2_out2, split2_out3] = split_tensors.try_into().unwrap();
        let squeeze4_out1 = split2_out1.squeeze_dims::<4>(&[2]);
        let squeeze5_out1 = split2_out2.squeeze_dims::<4>(&[2]);
        let squeeze6_out1 = split2_out3.squeeze_dims::<4>(&[2]);
        let unsqueeze22_out1: Tensor<4> = cos2_out1.unsqueeze_dims::<4>(&[1]);
        let unsqueeze23_out1: Tensor<4> = sin2_out1.unsqueeze_dims::<4>(&[1]);
        let mul18_out1 = squeeze4_out1.clone().mul(unsqueeze22_out1.clone());
        let shape15_out1: [i64; 4] = {
            let axes = &squeeze4_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather12_out1 = shape15_out1[3] as i64;
        let constant177_out1 = 2i64;
        let div5_out1 = gather12_out1 / constant177_out1;
        let unsqueeze24_out1 = [div5_out1 as i64];
        let slice8_out1 = squeeze4_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze24_out1[0]]);
        let unsqueeze25_out1 = [div5_out1 as i64];
        let slice9_out1 =
            squeeze4_out1.slice(s![.., .., .., unsqueeze25_out1[0]..9223372036854775807]);
        let neg3_out1 = slice9_out1.neg();
        let concat11_out1 = burn::tensor::Tensor::cat([neg3_out1, slice8_out1].into(), 3);
        let mul19_out1 = concat11_out1.mul(unsqueeze23_out1.clone());
        let add7_out1 = mul18_out1.add(mul19_out1);
        let mul20_out1 = squeeze5_out1.clone().mul(unsqueeze22_out1.clone());
        let shape16_out1: [i64; 4] = {
            let axes = &squeeze5_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather13_out1 = shape16_out1[3] as i64;
        let constant187_out1 = 2i64;
        let div6_out1 = gather13_out1 / constant187_out1;
        let unsqueeze26_out1 = [div6_out1 as i64];
        let slice10_out1 = squeeze5_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze26_out1[0]]);
        let unsqueeze27_out1 = [div6_out1 as i64];
        let slice11_out1 =
            squeeze5_out1.slice(s![.., .., .., unsqueeze27_out1[0]..9223372036854775807]);
        let neg4_out1 = slice11_out1.neg();
        let concat12_out1 = burn::tensor::Tensor::cat([neg4_out1, slice10_out1].into(), 3);
        let mul21_out1 = concat12_out1.mul(unsqueeze23_out1.clone());
        let add8_out1 = mul20_out1.add(mul21_out1);
        let shape17_out1: [i64; 4] = {
            let axes = &add7_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice12_out1: [i64; 1] = shape17_out1[3..4].try_into().unwrap();
        let cast29_out1 = {
            let shape_array = slice12_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt4_out1 = cast29_out1.sqrt();
        let constant198_out1 = self.constant198.val();
        let div7_out1 = constant198_out1.div(sqrt4_out1);
        let transpose8_out1 = add8_out1.permute([0, 1, 3, 2]);
        let sqrt5_out1 = div7_out1.sqrt();
        let mul22_out1 =
            add7_out1.mul((sqrt5_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul23_out1 =
            transpose8_out1.mul((sqrt5_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul10_out1 = mul22_out1.matmul(mul23_out1);
        let add9_out1 = matmul10_out1.add(where3_out1);
        let softmax2_out1 = burn::tensor::activation::softmax(add9_out1, 3);
        let matmul11_out1 = softmax2_out1.matmul(squeeze6_out1);
        let transpose9_out1 = matmul11_out1.permute([0, 2, 1, 3]);
        let unsqueeze28_out1 = [gather11_out1 as i64];
        let constant201_out1: [i64; 1] = [384i64];
        let constant200_out1: [i64; 1] = [-1i64];
        let concat13_out1: [i64; 3usize] = [
            &unsqueeze28_out1[..],
            &constant200_out1[..],
            &constant201_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape7_out1 = transpose9_out1.reshape(concat13_out1);
        let linear6_out1 = self.linear6.forward(reshape7_out1);
        let add10_out1 = add6_out1.add(linear6_out1);
        (add10_out1, unsqueeze22_out1, unsqueeze23_out1)
    }
}
#[derive(Module, Debug)]
pub struct Submodule3 {
    layernormalization4: LayerNorm,
    linear7: Linear,
    linear8: Linear,
    layernormalization5: LayerNorm,
    linear9: Linear,
    constant242: burn::module::Param<Tensor<1>>,
    linear10: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule3 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization4 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear7 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear8 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization5 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear9 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant242: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear10 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization4,
            linear7,
            linear8,
            layernormalization5,
            linear9,
            constant242,
            linear10,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add10_out1: Tensor<3>,
        unsqueeze22_out1: Tensor<4>,
        unsqueeze23_out1: Tensor<4>,
        where3_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization4_out1 = {
            let dtype = add10_out1.clone().dtype();
            self.layernormalization4
                .forward(add10_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear7_out1 = self.linear7.forward(layernormalization4_out1);
        let shape18_out1: [i64; 3] = {
            let axes = &linear7_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant203_out1: [i64; 1] = [-1i64];
        let gather14_out1: [i64; 1usize] = constant203_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape18_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape18_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant205_out1: [i64; 1] = [1i64];
        let add11_out1 = {
            let __lhs = gather14_out1;
            let __rhs = constant205_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant206_out1: [i64; 1] = [2i64];
        let div8_out1 = {
            let __lhs = add11_out1;
            let __rhs = constant206_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice13_out1 = linear7_out1.clone().slice(s![.., .., 0..div8_out1[0]]);
        let constant208_out1: [i64; 1] = [2i64];
        let mul25_out1 = {
            let __lhs = div8_out1;
            let __rhs = constant208_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice14_out1 = linear7_out1.slice(s![.., .., div8_out1[0]..mul25_out1[0]]);
        let sigmoid2_out1 = burn::tensor::activation::sigmoid(slice13_out1.clone());
        let mul26_out1 = slice13_out1.mul(sigmoid2_out1);
        let mul27_out1 = mul26_out1.mul(slice14_out1);
        let linear8_out1 = self.linear8.forward(mul27_out1);
        let add12_out1 = add10_out1.add(linear8_out1);
        let layernormalization5_out1 = {
            let dtype = add12_out1.clone().dtype();
            self.layernormalization5
                .forward(add12_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear9_out1 = self.linear9.forward(layernormalization5_out1.clone());
        let shape19_out1: [i64; 3] = {
            let axes = &layernormalization5_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather15_out1 = shape19_out1[0] as i64;
        let unsqueeze29_out1 = [gather15_out1 as i64];
        let constant215_out1: [i64; 1] = [32i64];
        let constant212_out1: [i64; 1] = [-1i64];
        let constant213_out1: [i64; 1] = [3i64];
        let constant214_out1: [i64; 1] = [12i64];
        let concat14_out1: [i64; 5usize] = [
            &unsqueeze29_out1[..],
            &constant212_out1[..],
            &constant213_out1[..],
            &constant214_out1[..],
            &constant215_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape8_out1 = linear9_out1.reshape(concat14_out1);
        let transpose10_out1 = reshape8_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose10_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split3_out1, split3_out2, split3_out3] = split_tensors.try_into().unwrap();
        let squeeze7_out1 = split3_out1.squeeze_dims::<4>(&[2]);
        let squeeze8_out1 = split3_out2.squeeze_dims::<4>(&[2]);
        let squeeze9_out1 = split3_out3.squeeze_dims::<4>(&[2]);
        let mul28_out1 = squeeze7_out1.clone().mul(unsqueeze22_out1.clone());
        let shape20_out1: [i64; 4] = {
            let axes = &squeeze7_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather16_out1 = shape20_out1[3] as i64;
        let constant221_out1 = 2i64;
        let div9_out1 = gather16_out1 / constant221_out1;
        let unsqueeze30_out1 = [div9_out1 as i64];
        let slice15_out1 = squeeze7_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze30_out1[0]]);
        let unsqueeze31_out1 = [div9_out1 as i64];
        let slice16_out1 =
            squeeze7_out1.slice(s![.., .., .., unsqueeze31_out1[0]..9223372036854775807]);
        let neg5_out1 = slice16_out1.neg();
        let concat15_out1 = burn::tensor::Tensor::cat([neg5_out1, slice15_out1].into(), 3);
        let mul29_out1 = concat15_out1.mul(unsqueeze23_out1.clone());
        let add13_out1 = mul28_out1.add(mul29_out1);
        let mul30_out1 = squeeze8_out1.clone().mul(unsqueeze22_out1);
        let shape21_out1: [i64; 4] = {
            let axes = &squeeze8_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather17_out1 = shape21_out1[3] as i64;
        let constant231_out1 = 2i64;
        let div10_out1 = gather17_out1 / constant231_out1;
        let unsqueeze32_out1 = [div10_out1 as i64];
        let slice17_out1 = squeeze8_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze32_out1[0]]);
        let unsqueeze33_out1 = [div10_out1 as i64];
        let slice18_out1 =
            squeeze8_out1.slice(s![.., .., .., unsqueeze33_out1[0]..9223372036854775807]);
        let neg6_out1 = slice18_out1.neg();
        let concat16_out1 = burn::tensor::Tensor::cat([neg6_out1, slice17_out1].into(), 3);
        let mul31_out1 = concat16_out1.mul(unsqueeze23_out1);
        let add14_out1 = mul30_out1.add(mul31_out1);
        let shape22_out1: [i64; 4] = {
            let axes = &add13_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice19_out1: [i64; 1] = shape22_out1[3..4].try_into().unwrap();
        let cast35_out1 = {
            let shape_array = slice19_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt7_out1 = cast35_out1.sqrt();
        let constant242_out1 = self.constant242.val();
        let div11_out1 = constant242_out1.div(sqrt7_out1);
        let transpose11_out1 = add14_out1.permute([0, 1, 3, 2]);
        let sqrt8_out1 = div11_out1.sqrt();
        let mul32_out1 =
            add13_out1.mul((sqrt8_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul33_out1 =
            transpose11_out1.mul((sqrt8_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul16_out1 = mul32_out1.matmul(mul33_out1);
        let add15_out1 = matmul16_out1.add(where3_out1);
        let softmax3_out1 = burn::tensor::activation::softmax(add15_out1, 3);
        let matmul17_out1 = softmax3_out1.matmul(squeeze9_out1);
        let transpose12_out1 = matmul17_out1.permute([0, 2, 1, 3]);
        let unsqueeze34_out1 = [gather15_out1 as i64];
        let constant245_out1: [i64; 1] = [384i64];
        let constant244_out1: [i64; 1] = [-1i64];
        let concat17_out1: [i64; 3usize] = [
            &unsqueeze34_out1[..],
            &constant244_out1[..],
            &constant245_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape9_out1 = transpose12_out1.reshape(concat17_out1);
        let linear10_out1 = self.linear10.forward(reshape9_out1);
        let add16_out1 = add12_out1.add(linear10_out1);
        add16_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule4 {
    layernormalization6: LayerNorm,
    linear11: Linear,
    linear12: Linear,
    layernormalization7: LayerNorm,
    linear13: Linear,
    constant286: burn::module::Param<Tensor<1>>,
    linear14: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule4 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization6 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear11 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear12 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization7 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear13 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant286: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear14 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization6,
            linear11,
            linear12,
            layernormalization7,
            linear13,
            constant286,
            linear14,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add16_out1: Tensor<3>,
        unsqueeze13_out1: Tensor<4>,
        unsqueeze14_out1: Tensor<4>,
        where2_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization6_out1 = {
            let dtype = add16_out1.clone().dtype();
            self.layernormalization6
                .forward(add16_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear11_out1 = self.linear11.forward(layernormalization6_out1);
        let shape23_out1: [i64; 3] = {
            let axes = &linear11_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant247_out1: [i64; 1] = [-1i64];
        let gather18_out1: [i64; 1usize] = constant247_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape23_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape23_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant249_out1: [i64; 1] = [1i64];
        let add17_out1 = {
            let __lhs = gather18_out1;
            let __rhs = constant249_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant250_out1: [i64; 1] = [2i64];
        let div12_out1 = {
            let __lhs = add17_out1;
            let __rhs = constant250_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice20_out1 = linear11_out1.clone().slice(s![.., .., 0..div12_out1[0]]);
        let constant252_out1: [i64; 1] = [2i64];
        let mul35_out1 = {
            let __lhs = div12_out1;
            let __rhs = constant252_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice21_out1 = linear11_out1.slice(s![.., .., div12_out1[0]..mul35_out1[0]]);
        let sigmoid3_out1 = burn::tensor::activation::sigmoid(slice20_out1.clone());
        let mul36_out1 = slice20_out1.mul(sigmoid3_out1);
        let mul37_out1 = mul36_out1.mul(slice21_out1);
        let linear12_out1 = self.linear12.forward(mul37_out1);
        let add18_out1 = add16_out1.add(linear12_out1);
        let layernormalization7_out1 = {
            let dtype = add18_out1.clone().dtype();
            self.layernormalization7
                .forward(add18_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear13_out1 = self.linear13.forward(layernormalization7_out1.clone());
        let shape24_out1: [i64; 3] = {
            let axes = &layernormalization7_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather19_out1 = shape24_out1[0] as i64;
        let unsqueeze35_out1 = [gather19_out1 as i64];
        let constant259_out1: [i64; 1] = [32i64];
        let constant256_out1: [i64; 1] = [-1i64];
        let constant257_out1: [i64; 1] = [3i64];
        let constant258_out1: [i64; 1] = [12i64];
        let concat18_out1: [i64; 5usize] = [
            &unsqueeze35_out1[..],
            &constant256_out1[..],
            &constant257_out1[..],
            &constant258_out1[..],
            &constant259_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape10_out1 = linear13_out1.reshape(concat18_out1);
        let transpose13_out1 = reshape10_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose13_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split4_out1, split4_out2, split4_out3] = split_tensors.try_into().unwrap();
        let squeeze10_out1 = split4_out1.squeeze_dims::<4>(&[2]);
        let squeeze11_out1 = split4_out2.squeeze_dims::<4>(&[2]);
        let squeeze12_out1 = split4_out3.squeeze_dims::<4>(&[2]);
        let mul38_out1 = squeeze10_out1.clone().mul(unsqueeze13_out1.clone());
        let shape25_out1: [i64; 4] = {
            let axes = &squeeze10_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather20_out1 = shape25_out1[3] as i64;
        let constant265_out1 = 2i64;
        let div13_out1 = gather20_out1 / constant265_out1;
        let unsqueeze36_out1 = [div13_out1 as i64];
        let slice22_out1 = squeeze10_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze36_out1[0]]);
        let unsqueeze37_out1 = [div13_out1 as i64];
        let slice23_out1 =
            squeeze10_out1.slice(s![.., .., .., unsqueeze37_out1[0]..9223372036854775807]);
        let neg7_out1 = slice23_out1.neg();
        let concat19_out1 = burn::tensor::Tensor::cat([neg7_out1, slice22_out1].into(), 3);
        let mul39_out1 = concat19_out1.mul(unsqueeze14_out1.clone());
        let add19_out1 = mul38_out1.add(mul39_out1);
        let mul40_out1 = squeeze11_out1.clone().mul(unsqueeze13_out1);
        let shape26_out1: [i64; 4] = {
            let axes = &squeeze11_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather21_out1 = shape26_out1[3] as i64;
        let constant275_out1 = 2i64;
        let div14_out1 = gather21_out1 / constant275_out1;
        let unsqueeze38_out1 = [div14_out1 as i64];
        let slice24_out1 = squeeze11_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze38_out1[0]]);
        let unsqueeze39_out1 = [div14_out1 as i64];
        let slice25_out1 =
            squeeze11_out1.slice(s![.., .., .., unsqueeze39_out1[0]..9223372036854775807]);
        let neg8_out1 = slice25_out1.neg();
        let concat20_out1 = burn::tensor::Tensor::cat([neg8_out1, slice24_out1].into(), 3);
        let mul41_out1 = concat20_out1.mul(unsqueeze14_out1);
        let add20_out1 = mul40_out1.add(mul41_out1);
        let shape27_out1: [i64; 4] = {
            let axes = &add19_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice26_out1: [i64; 1] = shape27_out1[3..4].try_into().unwrap();
        let cast41_out1 = {
            let shape_array = slice26_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt10_out1 = cast41_out1.sqrt();
        let constant286_out1 = self.constant286.val();
        let div15_out1 = constant286_out1.div(sqrt10_out1);
        let transpose14_out1 = add20_out1.permute([0, 1, 3, 2]);
        let sqrt11_out1 = div15_out1.sqrt();
        let mul42_out1 =
            add19_out1.mul((sqrt11_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul43_out1 =
            transpose14_out1.mul((sqrt11_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul22_out1 = mul42_out1.matmul(mul43_out1);
        let add21_out1 = matmul22_out1.add(where2_out1);
        let softmax4_out1 = burn::tensor::activation::softmax(add21_out1, 3);
        let matmul23_out1 = softmax4_out1.matmul(squeeze12_out1);
        let transpose15_out1 = matmul23_out1.permute([0, 2, 1, 3]);
        let unsqueeze40_out1 = [gather19_out1 as i64];
        let constant289_out1: [i64; 1] = [384i64];
        let constant288_out1: [i64; 1] = [-1i64];
        let concat21_out1: [i64; 3usize] = [
            &unsqueeze40_out1[..],
            &constant288_out1[..],
            &constant289_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape11_out1 = transpose15_out1.reshape(concat21_out1);
        let linear14_out1 = self.linear14.forward(reshape11_out1);
        let add22_out1 = add18_out1.add(linear14_out1);
        add22_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule5 {
    layernormalization8: LayerNorm,
    linear15: Linear,
    linear16: Linear,
    layernormalization9: LayerNorm,
    linear17: Linear,
    constant330: burn::module::Param<Tensor<1>>,
    linear18: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule5 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization8 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear15 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear16 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization9 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear17 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant330: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear18 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization8,
            linear15,
            linear16,
            layernormalization9,
            linear17,
            constant330,
            linear18,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add22_out1: Tensor<3>,
        unsqueeze22_out1: Tensor<4>,
        unsqueeze23_out1: Tensor<4>,
        where3_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization8_out1 = {
            let dtype = add22_out1.clone().dtype();
            self.layernormalization8
                .forward(add22_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear15_out1 = self.linear15.forward(layernormalization8_out1);
        let shape28_out1: [i64; 3] = {
            let axes = &linear15_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant291_out1: [i64; 1] = [-1i64];
        let gather22_out1: [i64; 1usize] = constant291_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape28_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape28_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant293_out1: [i64; 1] = [1i64];
        let add23_out1 = {
            let __lhs = gather22_out1;
            let __rhs = constant293_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant294_out1: [i64; 1] = [2i64];
        let div16_out1 = {
            let __lhs = add23_out1;
            let __rhs = constant294_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice27_out1 = linear15_out1.clone().slice(s![.., .., 0..div16_out1[0]]);
        let constant296_out1: [i64; 1] = [2i64];
        let mul45_out1 = {
            let __lhs = div16_out1;
            let __rhs = constant296_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice28_out1 = linear15_out1.slice(s![.., .., div16_out1[0]..mul45_out1[0]]);
        let sigmoid4_out1 = burn::tensor::activation::sigmoid(slice27_out1.clone());
        let mul46_out1 = slice27_out1.mul(sigmoid4_out1);
        let mul47_out1 = mul46_out1.mul(slice28_out1);
        let linear16_out1 = self.linear16.forward(mul47_out1);
        let add24_out1 = add22_out1.add(linear16_out1);
        let layernormalization9_out1 = {
            let dtype = add24_out1.clone().dtype();
            self.layernormalization9
                .forward(add24_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear17_out1 = self.linear17.forward(layernormalization9_out1.clone());
        let shape29_out1: [i64; 3] = {
            let axes = &layernormalization9_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather23_out1 = shape29_out1[0] as i64;
        let unsqueeze41_out1 = [gather23_out1 as i64];
        let constant303_out1: [i64; 1] = [32i64];
        let constant300_out1: [i64; 1] = [-1i64];
        let constant301_out1: [i64; 1] = [3i64];
        let constant302_out1: [i64; 1] = [12i64];
        let concat22_out1: [i64; 5usize] = [
            &unsqueeze41_out1[..],
            &constant300_out1[..],
            &constant301_out1[..],
            &constant302_out1[..],
            &constant303_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape12_out1 = linear17_out1.reshape(concat22_out1);
        let transpose16_out1 = reshape12_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose16_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split5_out1, split5_out2, split5_out3] = split_tensors.try_into().unwrap();
        let squeeze13_out1 = split5_out1.squeeze_dims::<4>(&[2]);
        let squeeze14_out1 = split5_out2.squeeze_dims::<4>(&[2]);
        let squeeze15_out1 = split5_out3.squeeze_dims::<4>(&[2]);
        let mul48_out1 = squeeze13_out1.clone().mul(unsqueeze22_out1.clone());
        let shape30_out1: [i64; 4] = {
            let axes = &squeeze13_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather24_out1 = shape30_out1[3] as i64;
        let constant309_out1 = 2i64;
        let div17_out1 = gather24_out1 / constant309_out1;
        let unsqueeze42_out1 = [div17_out1 as i64];
        let slice29_out1 = squeeze13_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze42_out1[0]]);
        let unsqueeze43_out1 = [div17_out1 as i64];
        let slice30_out1 =
            squeeze13_out1.slice(s![.., .., .., unsqueeze43_out1[0]..9223372036854775807]);
        let neg9_out1 = slice30_out1.neg();
        let concat23_out1 = burn::tensor::Tensor::cat([neg9_out1, slice29_out1].into(), 3);
        let mul49_out1 = concat23_out1.mul(unsqueeze23_out1.clone());
        let add25_out1 = mul48_out1.add(mul49_out1);
        let mul50_out1 = squeeze14_out1.clone().mul(unsqueeze22_out1);
        let shape31_out1: [i64; 4] = {
            let axes = &squeeze14_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather25_out1 = shape31_out1[3] as i64;
        let constant319_out1 = 2i64;
        let div18_out1 = gather25_out1 / constant319_out1;
        let unsqueeze44_out1 = [div18_out1 as i64];
        let slice31_out1 = squeeze14_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze44_out1[0]]);
        let unsqueeze45_out1 = [div18_out1 as i64];
        let slice32_out1 =
            squeeze14_out1.slice(s![.., .., .., unsqueeze45_out1[0]..9223372036854775807]);
        let neg10_out1 = slice32_out1.neg();
        let concat24_out1 = burn::tensor::Tensor::cat([neg10_out1, slice31_out1].into(), 3);
        let mul51_out1 = concat24_out1.mul(unsqueeze23_out1);
        let add26_out1 = mul50_out1.add(mul51_out1);
        let shape32_out1: [i64; 4] = {
            let axes = &add25_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice33_out1: [i64; 1] = shape32_out1[3..4].try_into().unwrap();
        let cast47_out1 = {
            let shape_array = slice33_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt13_out1 = cast47_out1.sqrt();
        let constant330_out1 = self.constant330.val();
        let div19_out1 = constant330_out1.div(sqrt13_out1);
        let transpose17_out1 = add26_out1.permute([0, 1, 3, 2]);
        let sqrt14_out1 = div19_out1.sqrt();
        let mul52_out1 =
            add25_out1.mul((sqrt14_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul53_out1 =
            transpose17_out1.mul((sqrt14_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul28_out1 = mul52_out1.matmul(mul53_out1);
        let add27_out1 = matmul28_out1.add(where3_out1);
        let softmax5_out1 = burn::tensor::activation::softmax(add27_out1, 3);
        let matmul29_out1 = softmax5_out1.matmul(squeeze15_out1);
        let transpose18_out1 = matmul29_out1.permute([0, 2, 1, 3]);
        let unsqueeze46_out1 = [gather23_out1 as i64];
        let constant333_out1: [i64; 1] = [384i64];
        let constant332_out1: [i64; 1] = [-1i64];
        let concat25_out1: [i64; 3usize] = [
            &unsqueeze46_out1[..],
            &constant332_out1[..],
            &constant333_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape13_out1 = transpose18_out1.reshape(concat25_out1);
        let linear18_out1 = self.linear18.forward(reshape13_out1);
        let add28_out1 = add24_out1.add(linear18_out1);
        add28_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule6 {
    layernormalization10: LayerNorm,
    linear19: Linear,
    linear20: Linear,
    layernormalization11: LayerNorm,
    linear21: Linear,
    constant374: burn::module::Param<Tensor<1>>,
    linear22: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule6 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization10 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear19 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear20 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization11 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear21 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant374: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear22 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization10,
            linear19,
            linear20,
            layernormalization11,
            linear21,
            constant374,
            linear22,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add28_out1: Tensor<3>,
        unsqueeze22_out1: Tensor<4>,
        unsqueeze23_out1: Tensor<4>,
        where3_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization10_out1 = {
            let dtype = add28_out1.clone().dtype();
            self.layernormalization10
                .forward(add28_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear19_out1 = self.linear19.forward(layernormalization10_out1);
        let shape33_out1: [i64; 3] = {
            let axes = &linear19_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant335_out1: [i64; 1] = [-1i64];
        let gather26_out1: [i64; 1usize] = constant335_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape33_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape33_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant337_out1: [i64; 1] = [1i64];
        let add29_out1 = {
            let __lhs = gather26_out1;
            let __rhs = constant337_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant338_out1: [i64; 1] = [2i64];
        let div20_out1 = {
            let __lhs = add29_out1;
            let __rhs = constant338_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice34_out1 = linear19_out1.clone().slice(s![.., .., 0..div20_out1[0]]);
        let constant340_out1: [i64; 1] = [2i64];
        let mul55_out1 = {
            let __lhs = div20_out1;
            let __rhs = constant340_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice35_out1 = linear19_out1.slice(s![.., .., div20_out1[0]..mul55_out1[0]]);
        let sigmoid5_out1 = burn::tensor::activation::sigmoid(slice34_out1.clone());
        let mul56_out1 = slice34_out1.mul(sigmoid5_out1);
        let mul57_out1 = mul56_out1.mul(slice35_out1);
        let linear20_out1 = self.linear20.forward(mul57_out1);
        let add30_out1 = add28_out1.add(linear20_out1);
        let layernormalization11_out1 = {
            let dtype = add30_out1.clone().dtype();
            self.layernormalization11
                .forward(add30_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear21_out1 = self.linear21.forward(layernormalization11_out1.clone());
        let shape34_out1: [i64; 3] = {
            let axes = &layernormalization11_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather27_out1 = shape34_out1[0] as i64;
        let unsqueeze47_out1 = [gather27_out1 as i64];
        let constant347_out1: [i64; 1] = [32i64];
        let constant344_out1: [i64; 1] = [-1i64];
        let constant345_out1: [i64; 1] = [3i64];
        let constant346_out1: [i64; 1] = [12i64];
        let concat26_out1: [i64; 5usize] = [
            &unsqueeze47_out1[..],
            &constant344_out1[..],
            &constant345_out1[..],
            &constant346_out1[..],
            &constant347_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape14_out1 = linear21_out1.reshape(concat26_out1);
        let transpose19_out1 = reshape14_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose19_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split6_out1, split6_out2, split6_out3] = split_tensors.try_into().unwrap();
        let squeeze16_out1 = split6_out1.squeeze_dims::<4>(&[2]);
        let squeeze17_out1 = split6_out2.squeeze_dims::<4>(&[2]);
        let squeeze18_out1 = split6_out3.squeeze_dims::<4>(&[2]);
        let mul58_out1 = squeeze16_out1.clone().mul(unsqueeze22_out1.clone());
        let shape35_out1: [i64; 4] = {
            let axes = &squeeze16_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather28_out1 = shape35_out1[3] as i64;
        let constant353_out1 = 2i64;
        let div21_out1 = gather28_out1 / constant353_out1;
        let unsqueeze48_out1 = [div21_out1 as i64];
        let slice36_out1 = squeeze16_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze48_out1[0]]);
        let unsqueeze49_out1 = [div21_out1 as i64];
        let slice37_out1 =
            squeeze16_out1.slice(s![.., .., .., unsqueeze49_out1[0]..9223372036854775807]);
        let neg11_out1 = slice37_out1.neg();
        let concat27_out1 = burn::tensor::Tensor::cat([neg11_out1, slice36_out1].into(), 3);
        let mul59_out1 = concat27_out1.mul(unsqueeze23_out1.clone());
        let add31_out1 = mul58_out1.add(mul59_out1);
        let mul60_out1 = squeeze17_out1.clone().mul(unsqueeze22_out1);
        let shape36_out1: [i64; 4] = {
            let axes = &squeeze17_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather29_out1 = shape36_out1[3] as i64;
        let constant363_out1 = 2i64;
        let div22_out1 = gather29_out1 / constant363_out1;
        let unsqueeze50_out1 = [div22_out1 as i64];
        let slice38_out1 = squeeze17_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze50_out1[0]]);
        let unsqueeze51_out1 = [div22_out1 as i64];
        let slice39_out1 =
            squeeze17_out1.slice(s![.., .., .., unsqueeze51_out1[0]..9223372036854775807]);
        let neg12_out1 = slice39_out1.neg();
        let concat28_out1 = burn::tensor::Tensor::cat([neg12_out1, slice38_out1].into(), 3);
        let mul61_out1 = concat28_out1.mul(unsqueeze23_out1);
        let add32_out1 = mul60_out1.add(mul61_out1);
        let shape37_out1: [i64; 4] = {
            let axes = &add31_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice40_out1: [i64; 1] = shape37_out1[3..4].try_into().unwrap();
        let cast53_out1 = {
            let shape_array = slice40_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt16_out1 = cast53_out1.sqrt();
        let constant374_out1 = self.constant374.val();
        let div23_out1 = constant374_out1.div(sqrt16_out1);
        let transpose20_out1 = add32_out1.permute([0, 1, 3, 2]);
        let sqrt17_out1 = div23_out1.sqrt();
        let mul62_out1 =
            add31_out1.mul((sqrt17_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul63_out1 =
            transpose20_out1.mul((sqrt17_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul34_out1 = mul62_out1.matmul(mul63_out1);
        let add33_out1 = matmul34_out1.add(where3_out1);
        let softmax6_out1 = burn::tensor::activation::softmax(add33_out1, 3);
        let matmul35_out1 = softmax6_out1.matmul(squeeze18_out1);
        let transpose21_out1 = matmul35_out1.permute([0, 2, 1, 3]);
        let unsqueeze52_out1 = [gather27_out1 as i64];
        let constant377_out1: [i64; 1] = [384i64];
        let constant376_out1: [i64; 1] = [-1i64];
        let concat29_out1: [i64; 3usize] = [
            &unsqueeze52_out1[..],
            &constant376_out1[..],
            &constant377_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape15_out1 = transpose21_out1.reshape(concat29_out1);
        let linear22_out1 = self.linear22.forward(reshape15_out1);
        let add34_out1 = add30_out1.add(linear22_out1);
        add34_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule7 {
    layernormalization12: LayerNorm,
    linear23: Linear,
    linear24: Linear,
    layernormalization13: LayerNorm,
    linear25: Linear,
    constant418: burn::module::Param<Tensor<1>>,
    linear26: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule7 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization12 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear23 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear24 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization13 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear25 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant418: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear26 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization12,
            linear23,
            linear24,
            layernormalization13,
            linear25,
            constant418,
            linear26,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add34_out1: Tensor<3>,
        unsqueeze13_out1: Tensor<4>,
        unsqueeze14_out1: Tensor<4>,
        where2_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization12_out1 = {
            let dtype = add34_out1.clone().dtype();
            self.layernormalization12
                .forward(add34_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear23_out1 = self.linear23.forward(layernormalization12_out1);
        let shape38_out1: [i64; 3] = {
            let axes = &linear23_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant379_out1: [i64; 1] = [-1i64];
        let gather30_out1: [i64; 1usize] = constant379_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape38_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape38_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant381_out1: [i64; 1] = [1i64];
        let add35_out1 = {
            let __lhs = gather30_out1;
            let __rhs = constant381_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant382_out1: [i64; 1] = [2i64];
        let div24_out1 = {
            let __lhs = add35_out1;
            let __rhs = constant382_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice41_out1 = linear23_out1.clone().slice(s![.., .., 0..div24_out1[0]]);
        let constant384_out1: [i64; 1] = [2i64];
        let mul65_out1 = {
            let __lhs = div24_out1;
            let __rhs = constant384_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice42_out1 = linear23_out1.slice(s![.., .., div24_out1[0]..mul65_out1[0]]);
        let sigmoid6_out1 = burn::tensor::activation::sigmoid(slice41_out1.clone());
        let mul66_out1 = slice41_out1.mul(sigmoid6_out1);
        let mul67_out1 = mul66_out1.mul(slice42_out1);
        let linear24_out1 = self.linear24.forward(mul67_out1);
        let add36_out1 = add34_out1.add(linear24_out1);
        let layernormalization13_out1 = {
            let dtype = add36_out1.clone().dtype();
            self.layernormalization13
                .forward(add36_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear25_out1 = self.linear25.forward(layernormalization13_out1.clone());
        let shape39_out1: [i64; 3] = {
            let axes = &layernormalization13_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather31_out1 = shape39_out1[0] as i64;
        let unsqueeze53_out1 = [gather31_out1 as i64];
        let constant391_out1: [i64; 1] = [32i64];
        let constant388_out1: [i64; 1] = [-1i64];
        let constant389_out1: [i64; 1] = [3i64];
        let constant390_out1: [i64; 1] = [12i64];
        let concat30_out1: [i64; 5usize] = [
            &unsqueeze53_out1[..],
            &constant388_out1[..],
            &constant389_out1[..],
            &constant390_out1[..],
            &constant391_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape16_out1 = linear25_out1.reshape(concat30_out1);
        let transpose22_out1 = reshape16_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose22_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split7_out1, split7_out2, split7_out3] = split_tensors.try_into().unwrap();
        let squeeze19_out1 = split7_out1.squeeze_dims::<4>(&[2]);
        let squeeze20_out1 = split7_out2.squeeze_dims::<4>(&[2]);
        let squeeze21_out1 = split7_out3.squeeze_dims::<4>(&[2]);
        let mul68_out1 = squeeze19_out1.clone().mul(unsqueeze13_out1.clone());
        let shape40_out1: [i64; 4] = {
            let axes = &squeeze19_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather32_out1 = shape40_out1[3] as i64;
        let constant397_out1 = 2i64;
        let div25_out1 = gather32_out1 / constant397_out1;
        let unsqueeze54_out1 = [div25_out1 as i64];
        let slice43_out1 = squeeze19_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze54_out1[0]]);
        let unsqueeze55_out1 = [div25_out1 as i64];
        let slice44_out1 =
            squeeze19_out1.slice(s![.., .., .., unsqueeze55_out1[0]..9223372036854775807]);
        let neg13_out1 = slice44_out1.neg();
        let concat31_out1 = burn::tensor::Tensor::cat([neg13_out1, slice43_out1].into(), 3);
        let mul69_out1 = concat31_out1.mul(unsqueeze14_out1.clone());
        let add37_out1 = mul68_out1.add(mul69_out1);
        let mul70_out1 = squeeze20_out1.clone().mul(unsqueeze13_out1);
        let shape41_out1: [i64; 4] = {
            let axes = &squeeze20_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather33_out1 = shape41_out1[3] as i64;
        let constant407_out1 = 2i64;
        let div26_out1 = gather33_out1 / constant407_out1;
        let unsqueeze56_out1 = [div26_out1 as i64];
        let slice45_out1 = squeeze20_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze56_out1[0]]);
        let unsqueeze57_out1 = [div26_out1 as i64];
        let slice46_out1 =
            squeeze20_out1.slice(s![.., .., .., unsqueeze57_out1[0]..9223372036854775807]);
        let neg14_out1 = slice46_out1.neg();
        let concat32_out1 = burn::tensor::Tensor::cat([neg14_out1, slice45_out1].into(), 3);
        let mul71_out1 = concat32_out1.mul(unsqueeze14_out1);
        let add38_out1 = mul70_out1.add(mul71_out1);
        let shape42_out1: [i64; 4] = {
            let axes = &add37_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice47_out1: [i64; 1] = shape42_out1[3..4].try_into().unwrap();
        let cast59_out1 = {
            let shape_array = slice47_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt19_out1 = cast59_out1.sqrt();
        let constant418_out1 = self.constant418.val();
        let div27_out1 = constant418_out1.div(sqrt19_out1);
        let transpose23_out1 = add38_out1.permute([0, 1, 3, 2]);
        let sqrt20_out1 = div27_out1.sqrt();
        let mul72_out1 =
            add37_out1.mul((sqrt20_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul73_out1 =
            transpose23_out1.mul((sqrt20_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul40_out1 = mul72_out1.matmul(mul73_out1);
        let add39_out1 = matmul40_out1.add(where2_out1);
        let softmax7_out1 = burn::tensor::activation::softmax(add39_out1, 3);
        let matmul41_out1 = softmax7_out1.matmul(squeeze21_out1);
        let transpose24_out1 = matmul41_out1.permute([0, 2, 1, 3]);
        let unsqueeze58_out1 = [gather31_out1 as i64];
        let constant421_out1: [i64; 1] = [384i64];
        let constant420_out1: [i64; 1] = [-1i64];
        let concat33_out1: [i64; 3usize] = [
            &unsqueeze58_out1[..],
            &constant420_out1[..],
            &constant421_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape17_out1 = transpose24_out1.reshape(concat33_out1);
        let linear26_out1 = self.linear26.forward(reshape17_out1);
        let add40_out1 = add36_out1.add(linear26_out1);
        add40_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule8 {
    layernormalization14: LayerNorm,
    linear27: Linear,
    linear28: Linear,
    layernormalization15: LayerNorm,
    linear29: Linear,
    constant462: burn::module::Param<Tensor<1>>,
    linear30: Linear,
    layernormalization16: LayerNorm,
    linear31: Linear,
    linear32: Linear,
    layernormalization17: LayerNorm,
    linear33: Linear,
    constant506: burn::module::Param<Tensor<1>>,
    linear34: Linear,
    layernormalization18: LayerNorm,
    linear35: Linear,
    linear36: Linear,
    layernormalization19: LayerNorm,
    linear37: Linear,
    constant550: burn::module::Param<Tensor<1>>,
    linear38: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule8 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization14 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear27 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear28 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization15 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear29 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant462: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear30 = LinearConfig::new(384, 384).with_bias(false).init(device);
        let layernormalization16 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear31 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear32 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization17 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear33 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant506: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear34 = LinearConfig::new(384, 384).with_bias(false).init(device);
        let layernormalization18 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear35 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear36 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization19 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear37 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant550: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear38 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization14,
            linear27,
            linear28,
            layernormalization15,
            linear29,
            constant462,
            linear30,
            layernormalization16,
            linear31,
            linear32,
            layernormalization17,
            linear33,
            constant506,
            linear34,
            layernormalization18,
            linear35,
            linear36,
            layernormalization19,
            linear37,
            constant550,
            linear38,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add40_out1: Tensor<3>,
        unsqueeze22_out1: Tensor<4>,
        unsqueeze23_out1: Tensor<4>,
        where3_out1: Tensor<4>,
        unsqueeze13_out1: Tensor<4>,
        unsqueeze14_out1: Tensor<4>,
        where2_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization14_out1 = {
            let dtype = add40_out1.clone().dtype();
            self.layernormalization14
                .forward(add40_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear27_out1 = self.linear27.forward(layernormalization14_out1);
        let shape43_out1: [i64; 3] = {
            let axes = &linear27_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant423_out1: [i64; 1] = [-1i64];
        let gather34_out1: [i64; 1usize] = constant423_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape43_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape43_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant425_out1: [i64; 1] = [1i64];
        let add41_out1 = {
            let __lhs = gather34_out1;
            let __rhs = constant425_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant426_out1: [i64; 1] = [2i64];
        let div28_out1 = {
            let __lhs = add41_out1;
            let __rhs = constant426_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice48_out1 = linear27_out1.clone().slice(s![.., .., 0..div28_out1[0]]);
        let constant428_out1: [i64; 1] = [2i64];
        let mul75_out1 = {
            let __lhs = div28_out1;
            let __rhs = constant428_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice49_out1 = linear27_out1.slice(s![.., .., div28_out1[0]..mul75_out1[0]]);
        let sigmoid7_out1 = burn::tensor::activation::sigmoid(slice48_out1.clone());
        let mul76_out1 = slice48_out1.mul(sigmoid7_out1);
        let mul77_out1 = mul76_out1.mul(slice49_out1);
        let linear28_out1 = self.linear28.forward(mul77_out1);
        let add42_out1 = add40_out1.add(linear28_out1);
        let layernormalization15_out1 = {
            let dtype = add42_out1.clone().dtype();
            self.layernormalization15
                .forward(add42_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear29_out1 = self.linear29.forward(layernormalization15_out1.clone());
        let shape44_out1: [i64; 3] = {
            let axes = &layernormalization15_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather35_out1 = shape44_out1[0] as i64;
        let unsqueeze59_out1 = [gather35_out1 as i64];
        let constant435_out1: [i64; 1] = [32i64];
        let constant432_out1: [i64; 1] = [-1i64];
        let constant433_out1: [i64; 1] = [3i64];
        let constant434_out1: [i64; 1] = [12i64];
        let concat34_out1: [i64; 5usize] = [
            &unsqueeze59_out1[..],
            &constant432_out1[..],
            &constant433_out1[..],
            &constant434_out1[..],
            &constant435_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape18_out1 = linear29_out1.reshape(concat34_out1);
        let transpose25_out1 = reshape18_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose25_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split8_out1, split8_out2, split8_out3] = split_tensors.try_into().unwrap();
        let squeeze22_out1 = split8_out1.squeeze_dims::<4>(&[2]);
        let squeeze23_out1 = split8_out2.squeeze_dims::<4>(&[2]);
        let squeeze24_out1 = split8_out3.squeeze_dims::<4>(&[2]);
        let mul78_out1 = squeeze22_out1.clone().mul(unsqueeze22_out1.clone());
        let shape45_out1: [i64; 4] = {
            let axes = &squeeze22_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather36_out1 = shape45_out1[3] as i64;
        let constant441_out1 = 2i64;
        let div29_out1 = gather36_out1 / constant441_out1;
        let unsqueeze60_out1 = [div29_out1 as i64];
        let slice50_out1 = squeeze22_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze60_out1[0]]);
        let unsqueeze61_out1 = [div29_out1 as i64];
        let slice51_out1 =
            squeeze22_out1.slice(s![.., .., .., unsqueeze61_out1[0]..9223372036854775807]);
        let neg15_out1 = slice51_out1.neg();
        let concat35_out1 = burn::tensor::Tensor::cat([neg15_out1, slice50_out1].into(), 3);
        let mul79_out1 = concat35_out1.mul(unsqueeze23_out1.clone());
        let add43_out1 = mul78_out1.add(mul79_out1);
        let mul80_out1 = squeeze23_out1.clone().mul(unsqueeze22_out1.clone());
        let shape46_out1: [i64; 4] = {
            let axes = &squeeze23_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather37_out1 = shape46_out1[3] as i64;
        let constant451_out1 = 2i64;
        let div30_out1 = gather37_out1 / constant451_out1;
        let unsqueeze62_out1 = [div30_out1 as i64];
        let slice52_out1 = squeeze23_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze62_out1[0]]);
        let unsqueeze63_out1 = [div30_out1 as i64];
        let slice53_out1 =
            squeeze23_out1.slice(s![.., .., .., unsqueeze63_out1[0]..9223372036854775807]);
        let neg16_out1 = slice53_out1.neg();
        let concat36_out1 = burn::tensor::Tensor::cat([neg16_out1, slice52_out1].into(), 3);
        let mul81_out1 = concat36_out1.mul(unsqueeze23_out1.clone());
        let add44_out1 = mul80_out1.add(mul81_out1);
        let shape47_out1: [i64; 4] = {
            let axes = &add43_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice54_out1: [i64; 1] = shape47_out1[3..4].try_into().unwrap();
        let cast65_out1 = {
            let shape_array = slice54_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt22_out1 = cast65_out1.sqrt();
        let constant462_out1 = self.constant462.val();
        let div31_out1 = constant462_out1.div(sqrt22_out1);
        let transpose26_out1 = add44_out1.permute([0, 1, 3, 2]);
        let sqrt23_out1 = div31_out1.sqrt();
        let mul82_out1 =
            add43_out1.mul((sqrt23_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul83_out1 =
            transpose26_out1.mul((sqrt23_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul46_out1 = mul82_out1.matmul(mul83_out1);
        let add45_out1 = matmul46_out1.add(where3_out1.clone());
        let softmax8_out1 = burn::tensor::activation::softmax(add45_out1, 3);
        let matmul47_out1 = softmax8_out1.matmul(squeeze24_out1);
        let transpose27_out1 = matmul47_out1.permute([0, 2, 1, 3]);
        let unsqueeze64_out1 = [gather35_out1 as i64];
        let constant465_out1: [i64; 1] = [384i64];
        let constant464_out1: [i64; 1] = [-1i64];
        let concat37_out1: [i64; 3usize] = [
            &unsqueeze64_out1[..],
            &constant464_out1[..],
            &constant465_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape19_out1 = transpose27_out1.reshape(concat37_out1);
        let linear30_out1 = self.linear30.forward(reshape19_out1);
        let add46_out1 = add42_out1.add(linear30_out1);
        let layernormalization16_out1 = {
            let dtype = add46_out1.clone().dtype();
            self.layernormalization16
                .forward(add46_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear31_out1 = self.linear31.forward(layernormalization16_out1);
        let shape48_out1: [i64; 3] = {
            let axes = &linear31_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant467_out1: [i64; 1] = [-1i64];
        let gather38_out1: [i64; 1usize] = constant467_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape48_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape48_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant469_out1: [i64; 1] = [1i64];
        let add47_out1 = {
            let __lhs = gather38_out1;
            let __rhs = constant469_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant470_out1: [i64; 1] = [2i64];
        let div32_out1 = {
            let __lhs = add47_out1;
            let __rhs = constant470_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice55_out1 = linear31_out1.clone().slice(s![.., .., 0..div32_out1[0]]);
        let constant472_out1: [i64; 1] = [2i64];
        let mul85_out1 = {
            let __lhs = div32_out1;
            let __rhs = constant472_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice56_out1 = linear31_out1.slice(s![.., .., div32_out1[0]..mul85_out1[0]]);
        let sigmoid8_out1 = burn::tensor::activation::sigmoid(slice55_out1.clone());
        let mul86_out1 = slice55_out1.mul(sigmoid8_out1);
        let mul87_out1 = mul86_out1.mul(slice56_out1);
        let linear32_out1 = self.linear32.forward(mul87_out1);
        let add48_out1 = add46_out1.add(linear32_out1);
        let layernormalization17_out1 = {
            let dtype = add48_out1.clone().dtype();
            self.layernormalization17
                .forward(add48_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear33_out1 = self.linear33.forward(layernormalization17_out1.clone());
        let shape49_out1: [i64; 3] = {
            let axes = &layernormalization17_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather39_out1 = shape49_out1[0] as i64;
        let unsqueeze65_out1 = [gather39_out1 as i64];
        let constant479_out1: [i64; 1] = [32i64];
        let constant476_out1: [i64; 1] = [-1i64];
        let constant477_out1: [i64; 1] = [3i64];
        let constant478_out1: [i64; 1] = [12i64];
        let concat38_out1: [i64; 5usize] = [
            &unsqueeze65_out1[..],
            &constant476_out1[..],
            &constant477_out1[..],
            &constant478_out1[..],
            &constant479_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape20_out1 = linear33_out1.reshape(concat38_out1);
        let transpose28_out1 = reshape20_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose28_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split9_out1, split9_out2, split9_out3] = split_tensors.try_into().unwrap();
        let squeeze25_out1 = split9_out1.squeeze_dims::<4>(&[2]);
        let squeeze26_out1 = split9_out2.squeeze_dims::<4>(&[2]);
        let squeeze27_out1 = split9_out3.squeeze_dims::<4>(&[2]);
        let mul88_out1 = squeeze25_out1.clone().mul(unsqueeze22_out1.clone());
        let shape50_out1: [i64; 4] = {
            let axes = &squeeze25_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather40_out1 = shape50_out1[3] as i64;
        let constant485_out1 = 2i64;
        let div33_out1 = gather40_out1 / constant485_out1;
        let unsqueeze66_out1 = [div33_out1 as i64];
        let slice57_out1 = squeeze25_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze66_out1[0]]);
        let unsqueeze67_out1 = [div33_out1 as i64];
        let slice58_out1 =
            squeeze25_out1.slice(s![.., .., .., unsqueeze67_out1[0]..9223372036854775807]);
        let neg17_out1 = slice58_out1.neg();
        let concat39_out1 = burn::tensor::Tensor::cat([neg17_out1, slice57_out1].into(), 3);
        let mul89_out1 = concat39_out1.mul(unsqueeze23_out1.clone());
        let add49_out1 = mul88_out1.add(mul89_out1);
        let mul90_out1 = squeeze26_out1.clone().mul(unsqueeze22_out1);
        let shape51_out1: [i64; 4] = {
            let axes = &squeeze26_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather41_out1 = shape51_out1[3] as i64;
        let constant495_out1 = 2i64;
        let div34_out1 = gather41_out1 / constant495_out1;
        let unsqueeze68_out1 = [div34_out1 as i64];
        let slice59_out1 = squeeze26_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze68_out1[0]]);
        let unsqueeze69_out1 = [div34_out1 as i64];
        let slice60_out1 =
            squeeze26_out1.slice(s![.., .., .., unsqueeze69_out1[0]..9223372036854775807]);
        let neg18_out1 = slice60_out1.neg();
        let concat40_out1 = burn::tensor::Tensor::cat([neg18_out1, slice59_out1].into(), 3);
        let mul91_out1 = concat40_out1.mul(unsqueeze23_out1);
        let add50_out1 = mul90_out1.add(mul91_out1);
        let shape52_out1: [i64; 4] = {
            let axes = &add49_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice61_out1: [i64; 1] = shape52_out1[3..4].try_into().unwrap();
        let cast71_out1 = {
            let shape_array = slice61_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt25_out1 = cast71_out1.sqrt();
        let constant506_out1 = self.constant506.val();
        let div35_out1 = constant506_out1.div(sqrt25_out1);
        let transpose29_out1 = add50_out1.permute([0, 1, 3, 2]);
        let sqrt26_out1 = div35_out1.sqrt();
        let mul92_out1 =
            add49_out1.mul((sqrt26_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul93_out1 =
            transpose29_out1.mul((sqrt26_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul52_out1 = mul92_out1.matmul(mul93_out1);
        let add51_out1 = matmul52_out1.add(where3_out1);
        let softmax9_out1 = burn::tensor::activation::softmax(add51_out1, 3);
        let matmul53_out1 = softmax9_out1.matmul(squeeze27_out1);
        let transpose30_out1 = matmul53_out1.permute([0, 2, 1, 3]);
        let unsqueeze70_out1 = [gather39_out1 as i64];
        let constant509_out1: [i64; 1] = [384i64];
        let constant508_out1: [i64; 1] = [-1i64];
        let concat41_out1: [i64; 3usize] = [
            &unsqueeze70_out1[..],
            &constant508_out1[..],
            &constant509_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape21_out1 = transpose30_out1.reshape(concat41_out1);
        let linear34_out1 = self.linear34.forward(reshape21_out1);
        let add52_out1 = add48_out1.add(linear34_out1);
        let layernormalization18_out1 = {
            let dtype = add52_out1.clone().dtype();
            self.layernormalization18
                .forward(add52_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear35_out1 = self.linear35.forward(layernormalization18_out1);
        let shape53_out1: [i64; 3] = {
            let axes = &linear35_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant511_out1: [i64; 1] = [-1i64];
        let gather42_out1: [i64; 1usize] = constant511_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape53_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape53_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant513_out1: [i64; 1] = [1i64];
        let add53_out1 = {
            let __lhs = gather42_out1;
            let __rhs = constant513_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant514_out1: [i64; 1] = [2i64];
        let div36_out1 = {
            let __lhs = add53_out1;
            let __rhs = constant514_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice62_out1 = linear35_out1.clone().slice(s![.., .., 0..div36_out1[0]]);
        let constant516_out1: [i64; 1] = [2i64];
        let mul95_out1 = {
            let __lhs = div36_out1;
            let __rhs = constant516_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice63_out1 = linear35_out1.slice(s![.., .., div36_out1[0]..mul95_out1[0]]);
        let sigmoid9_out1 = burn::tensor::activation::sigmoid(slice62_out1.clone());
        let mul96_out1 = slice62_out1.mul(sigmoid9_out1);
        let mul97_out1 = mul96_out1.mul(slice63_out1);
        let linear36_out1 = self.linear36.forward(mul97_out1);
        let add54_out1 = add52_out1.add(linear36_out1);
        let layernormalization19_out1 = {
            let dtype = add54_out1.clone().dtype();
            self.layernormalization19
                .forward(add54_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear37_out1 = self.linear37.forward(layernormalization19_out1.clone());
        let shape54_out1: [i64; 3] = {
            let axes = &layernormalization19_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather43_out1 = shape54_out1[0] as i64;
        let unsqueeze71_out1 = [gather43_out1 as i64];
        let constant523_out1: [i64; 1] = [32i64];
        let constant520_out1: [i64; 1] = [-1i64];
        let constant521_out1: [i64; 1] = [3i64];
        let constant522_out1: [i64; 1] = [12i64];
        let concat42_out1: [i64; 5usize] = [
            &unsqueeze71_out1[..],
            &constant520_out1[..],
            &constant521_out1[..],
            &constant522_out1[..],
            &constant523_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape22_out1 = linear37_out1.reshape(concat42_out1);
        let transpose31_out1 = reshape22_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose31_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split10_out1, split10_out2, split10_out3] = split_tensors.try_into().unwrap();
        let squeeze28_out1 = split10_out1.squeeze_dims::<4>(&[2]);
        let squeeze29_out1 = split10_out2.squeeze_dims::<4>(&[2]);
        let squeeze30_out1 = split10_out3.squeeze_dims::<4>(&[2]);
        let mul98_out1 = squeeze28_out1.clone().mul(unsqueeze13_out1.clone());
        let shape55_out1: [i64; 4] = {
            let axes = &squeeze28_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather44_out1 = shape55_out1[3] as i64;
        let constant529_out1 = 2i64;
        let div37_out1 = gather44_out1 / constant529_out1;
        let unsqueeze72_out1 = [div37_out1 as i64];
        let slice64_out1 = squeeze28_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze72_out1[0]]);
        let unsqueeze73_out1 = [div37_out1 as i64];
        let slice65_out1 =
            squeeze28_out1.slice(s![.., .., .., unsqueeze73_out1[0]..9223372036854775807]);
        let neg19_out1 = slice65_out1.neg();
        let concat43_out1 = burn::tensor::Tensor::cat([neg19_out1, slice64_out1].into(), 3);
        let mul99_out1 = concat43_out1.mul(unsqueeze14_out1.clone());
        let add55_out1 = mul98_out1.add(mul99_out1);
        let mul100_out1 = squeeze29_out1.clone().mul(unsqueeze13_out1);
        let shape56_out1: [i64; 4] = {
            let axes = &squeeze29_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather45_out1 = shape56_out1[3] as i64;
        let constant539_out1 = 2i64;
        let div38_out1 = gather45_out1 / constant539_out1;
        let unsqueeze74_out1 = [div38_out1 as i64];
        let slice66_out1 = squeeze29_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze74_out1[0]]);
        let unsqueeze75_out1 = [div38_out1 as i64];
        let slice67_out1 =
            squeeze29_out1.slice(s![.., .., .., unsqueeze75_out1[0]..9223372036854775807]);
        let neg20_out1 = slice67_out1.neg();
        let concat44_out1 = burn::tensor::Tensor::cat([neg20_out1, slice66_out1].into(), 3);
        let mul101_out1 = concat44_out1.mul(unsqueeze14_out1);
        let add56_out1 = mul100_out1.add(mul101_out1);
        let shape57_out1: [i64; 4] = {
            let axes = &add55_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice68_out1: [i64; 1] = shape57_out1[3..4].try_into().unwrap();
        let cast77_out1 = {
            let shape_array = slice68_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt28_out1 = cast77_out1.sqrt();
        let constant550_out1 = self.constant550.val();
        let div39_out1 = constant550_out1.div(sqrt28_out1);
        let transpose32_out1 = add56_out1.permute([0, 1, 3, 2]);
        let sqrt29_out1 = div39_out1.sqrt();
        let mul102_out1 =
            add55_out1.mul((sqrt29_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul103_out1 =
            transpose32_out1.mul((sqrt29_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul58_out1 = mul102_out1.matmul(mul103_out1);
        let add57_out1 = matmul58_out1.add(where2_out1);
        let softmax10_out1 = burn::tensor::activation::softmax(add57_out1, 3);
        let matmul59_out1 = softmax10_out1.matmul(squeeze30_out1);
        let transpose33_out1 = matmul59_out1.permute([0, 2, 1, 3]);
        let unsqueeze76_out1 = [gather43_out1 as i64];
        let constant553_out1: [i64; 1] = [384i64];
        let constant552_out1: [i64; 1] = [-1i64];
        let concat45_out1: [i64; 3usize] = [
            &unsqueeze76_out1[..],
            &constant552_out1[..],
            &constant553_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape23_out1 = transpose33_out1.reshape(concat45_out1);
        let linear38_out1 = self.linear38.forward(reshape23_out1);
        let add58_out1 = add54_out1.add(linear38_out1);
        add58_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule9 {
    layernormalization20: LayerNorm,
    linear39: Linear,
    linear40: Linear,
    layernormalization21: LayerNorm,
    linear41: Linear,
    constant594: burn::module::Param<Tensor<1>>,
    linear42: Linear,
    #[module(skip)]
    device: Device,
}
impl Submodule9 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization20 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear39 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear40 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization21 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear41 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant594: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear42 = LinearConfig::new(384, 384).with_bias(false).init(device);
        Self {
            layernormalization20,
            linear39,
            linear40,
            layernormalization21,
            linear41,
            constant594,
            linear42,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add58_out1: Tensor<3>,
        unsqueeze22_out1: Tensor<4>,
        unsqueeze23_out1: Tensor<4>,
        where3_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization20_out1 = {
            let dtype = add58_out1.clone().dtype();
            self.layernormalization20
                .forward(add58_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear39_out1 = self.linear39.forward(layernormalization20_out1);
        let shape58_out1: [i64; 3] = {
            let axes = &linear39_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant555_out1: [i64; 1] = [-1i64];
        let gather46_out1: [i64; 1usize] = constant555_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape58_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape58_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant557_out1: [i64; 1] = [1i64];
        let add59_out1 = {
            let __lhs = gather46_out1;
            let __rhs = constant557_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant558_out1: [i64; 1] = [2i64];
        let div40_out1 = {
            let __lhs = add59_out1;
            let __rhs = constant558_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice69_out1 = linear39_out1.clone().slice(s![.., .., 0..div40_out1[0]]);
        let constant560_out1: [i64; 1] = [2i64];
        let mul105_out1 = {
            let __lhs = div40_out1;
            let __rhs = constant560_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice70_out1 = linear39_out1.slice(s![.., .., div40_out1[0]..mul105_out1[0]]);
        let sigmoid10_out1 = burn::tensor::activation::sigmoid(slice69_out1.clone());
        let mul106_out1 = slice69_out1.mul(sigmoid10_out1);
        let mul107_out1 = mul106_out1.mul(slice70_out1);
        let linear40_out1 = self.linear40.forward(mul107_out1);
        let add60_out1 = add58_out1.add(linear40_out1);
        let layernormalization21_out1 = {
            let dtype = add60_out1.clone().dtype();
            self.layernormalization21
                .forward(add60_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear41_out1 = self.linear41.forward(layernormalization21_out1.clone());
        let shape59_out1: [i64; 3] = {
            let axes = &layernormalization21_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather47_out1 = shape59_out1[0] as i64;
        let unsqueeze77_out1 = [gather47_out1 as i64];
        let constant567_out1: [i64; 1] = [32i64];
        let constant564_out1: [i64; 1] = [-1i64];
        let constant565_out1: [i64; 1] = [3i64];
        let constant566_out1: [i64; 1] = [12i64];
        let concat46_out1: [i64; 5usize] = [
            &unsqueeze77_out1[..],
            &constant564_out1[..],
            &constant565_out1[..],
            &constant566_out1[..],
            &constant567_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape24_out1 = linear41_out1.reshape(concat46_out1);
        let transpose34_out1 = reshape24_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose34_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split11_out1, split11_out2, split11_out3] = split_tensors.try_into().unwrap();
        let squeeze31_out1 = split11_out1.squeeze_dims::<4>(&[2]);
        let squeeze32_out1 = split11_out2.squeeze_dims::<4>(&[2]);
        let squeeze33_out1 = split11_out3.squeeze_dims::<4>(&[2]);
        let mul108_out1 = squeeze31_out1.clone().mul(unsqueeze22_out1.clone());
        let shape60_out1: [i64; 4] = {
            let axes = &squeeze31_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather48_out1 = shape60_out1[3] as i64;
        let constant573_out1 = 2i64;
        let div41_out1 = gather48_out1 / constant573_out1;
        let unsqueeze78_out1 = [div41_out1 as i64];
        let slice71_out1 = squeeze31_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze78_out1[0]]);
        let unsqueeze79_out1 = [div41_out1 as i64];
        let slice72_out1 =
            squeeze31_out1.slice(s![.., .., .., unsqueeze79_out1[0]..9223372036854775807]);
        let neg21_out1 = slice72_out1.neg();
        let concat47_out1 = burn::tensor::Tensor::cat([neg21_out1, slice71_out1].into(), 3);
        let mul109_out1 = concat47_out1.mul(unsqueeze23_out1.clone());
        let add61_out1 = mul108_out1.add(mul109_out1);
        let mul110_out1 = squeeze32_out1.clone().mul(unsqueeze22_out1);
        let shape61_out1: [i64; 4] = {
            let axes = &squeeze32_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather49_out1 = shape61_out1[3] as i64;
        let constant583_out1 = 2i64;
        let div42_out1 = gather49_out1 / constant583_out1;
        let unsqueeze80_out1 = [div42_out1 as i64];
        let slice73_out1 = squeeze32_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze80_out1[0]]);
        let unsqueeze81_out1 = [div42_out1 as i64];
        let slice74_out1 =
            squeeze32_out1.slice(s![.., .., .., unsqueeze81_out1[0]..9223372036854775807]);
        let neg22_out1 = slice74_out1.neg();
        let concat48_out1 = burn::tensor::Tensor::cat([neg22_out1, slice73_out1].into(), 3);
        let mul111_out1 = concat48_out1.mul(unsqueeze23_out1);
        let add62_out1 = mul110_out1.add(mul111_out1);
        let shape62_out1: [i64; 4] = {
            let axes = &add61_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice75_out1: [i64; 1] = shape62_out1[3..4].try_into().unwrap();
        let cast83_out1 = {
            let shape_array = slice75_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt31_out1 = cast83_out1.sqrt();
        let constant594_out1 = self.constant594.val();
        let div43_out1 = constant594_out1.div(sqrt31_out1);
        let transpose35_out1 = add62_out1.permute([0, 1, 3, 2]);
        let sqrt32_out1 = div43_out1.sqrt();
        let mul112_out1 =
            add61_out1.mul((sqrt32_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul113_out1 =
            transpose35_out1.mul((sqrt32_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul64_out1 = mul112_out1.matmul(mul113_out1);
        let add63_out1 = matmul64_out1.add(where3_out1);
        let softmax11_out1 = burn::tensor::activation::softmax(add63_out1, 3);
        let matmul65_out1 = softmax11_out1.matmul(squeeze33_out1);
        let transpose36_out1 = matmul65_out1.permute([0, 2, 1, 3]);
        let unsqueeze82_out1 = [gather47_out1 as i64];
        let constant597_out1: [i64; 1] = [384i64];
        let constant596_out1: [i64; 1] = [-1i64];
        let concat49_out1: [i64; 3usize] = [
            &unsqueeze82_out1[..],
            &constant596_out1[..],
            &constant597_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape25_out1 = transpose36_out1.reshape(concat49_out1);
        let linear42_out1 = self.linear42.forward(reshape25_out1);
        let add64_out1 = add60_out1.add(linear42_out1);
        add64_out1
    }
}
#[derive(Module, Debug)]
pub struct Submodule10 {
    layernormalization22: LayerNorm,
    linear43: Linear,
    linear44: Linear,
    layernormalization23: LayerNorm,
    linear45: Linear,
    constant638: burn::module::Param<Tensor<1>>,
    linear46: Linear,
    layernormalization24: LayerNorm,
    linear47: Linear,
    linear48: Linear,
    layernormalization25: LayerNorm,
    #[module(skip)]
    device: Device,
}
impl Submodule10 {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let layernormalization22 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear43 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear44 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization23 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear45 = LinearConfig::new(384, 1152).with_bias(false).init(device);
        let constant638: burn::module::Param<Tensor<1>> = burn::module::Param::uninitialized(
            burn::module::ParamId::new(),
            move |device, _require_grad| {
                Tensor::<1>::zeros([1], (device, burn::tensor::DType::F32))
            },
            device.clone(),
            false,
            [1].into(),
        );
        let linear46 = LinearConfig::new(384, 384).with_bias(false).init(device);
        let layernormalization24 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        let linear47 = LinearConfig::new(384, 3072).with_bias(false).init(device);
        let linear48 = LinearConfig::new(1536, 384).with_bias(false).init(device);
        let layernormalization25 = LayerNormConfig::new(384)
            .with_epsilon(0.000009999999747378752f64)
            .with_bias(true)
            .init(device);
        Self {
            layernormalization22,
            linear43,
            linear44,
            layernormalization23,
            linear45,
            constant638,
            linear46,
            layernormalization24,
            linear47,
            linear48,
            layernormalization25,
            device: device.clone(),
        }
    }
    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(
        &self,
        add64_out1: Tensor<3>,
        unsqueeze22_out1: Tensor<4>,
        unsqueeze23_out1: Tensor<4>,
        where3_out1: Tensor<4>,
    ) -> Tensor<3> {
        let layernormalization22_out1 = {
            let dtype = add64_out1.clone().dtype();
            self.layernormalization22
                .forward(add64_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear43_out1 = self.linear43.forward(layernormalization22_out1);
        let shape63_out1: [i64; 3] = {
            let axes = &linear43_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant599_out1: [i64; 1] = [-1i64];
        let gather50_out1: [i64; 1usize] = constant599_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape63_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape63_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant601_out1: [i64; 1] = [1i64];
        let add65_out1 = {
            let __lhs = gather50_out1;
            let __rhs = constant601_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant602_out1: [i64; 1] = [2i64];
        let div44_out1 = {
            let __lhs = add65_out1;
            let __rhs = constant602_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice76_out1 = linear43_out1.clone().slice(s![.., .., 0..div44_out1[0]]);
        let constant604_out1: [i64; 1] = [2i64];
        let mul115_out1 = {
            let __lhs = div44_out1;
            let __rhs = constant604_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice77_out1 = linear43_out1.slice(s![.., .., div44_out1[0]..mul115_out1[0]]);
        let sigmoid11_out1 = burn::tensor::activation::sigmoid(slice76_out1.clone());
        let mul116_out1 = slice76_out1.mul(sigmoid11_out1);
        let mul117_out1 = mul116_out1.mul(slice77_out1);
        let linear44_out1 = self.linear44.forward(mul117_out1);
        let add66_out1 = add64_out1.add(linear44_out1);
        let layernormalization23_out1 = {
            let dtype = add66_out1.clone().dtype();
            self.layernormalization23
                .forward(add66_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear45_out1 = self.linear45.forward(layernormalization23_out1.clone());
        let shape64_out1: [i64; 3] = {
            let axes = &layernormalization23_out1.dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather51_out1 = shape64_out1[0] as i64;
        let unsqueeze83_out1 = [gather51_out1 as i64];
        let constant611_out1: [i64; 1] = [32i64];
        let constant608_out1: [i64; 1] = [-1i64];
        let constant609_out1: [i64; 1] = [3i64];
        let constant610_out1: [i64; 1] = [12i64];
        let concat50_out1: [i64; 5usize] = [
            &unsqueeze83_out1[..],
            &constant608_out1[..],
            &constant609_out1[..],
            &constant610_out1[..],
            &constant611_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape26_out1 = linear45_out1.reshape(concat50_out1);
        let transpose37_out1 = reshape26_out1.permute([0, 3, 2, 1, 4]);
        let split_tensors = transpose37_out1.split_with_sizes([1, 1, 1].into(), 2);
        let [split12_out1, split12_out2, split12_out3] = split_tensors.try_into().unwrap();
        let squeeze34_out1 = split12_out1.squeeze_dims::<4>(&[2]);
        let squeeze35_out1 = split12_out2.squeeze_dims::<4>(&[2]);
        let squeeze36_out1 = split12_out3.squeeze_dims::<4>(&[2]);
        let mul118_out1 = squeeze34_out1.clone().mul(unsqueeze22_out1.clone());
        let shape65_out1: [i64; 4] = {
            let axes = &squeeze34_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather52_out1 = shape65_out1[3] as i64;
        let constant617_out1 = 2i64;
        let div45_out1 = gather52_out1 / constant617_out1;
        let unsqueeze84_out1 = [div45_out1 as i64];
        let slice78_out1 = squeeze34_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze84_out1[0]]);
        let unsqueeze85_out1 = [div45_out1 as i64];
        let slice79_out1 =
            squeeze34_out1.slice(s![.., .., .., unsqueeze85_out1[0]..9223372036854775807]);
        let neg23_out1 = slice79_out1.neg();
        let concat51_out1 = burn::tensor::Tensor::cat([neg23_out1, slice78_out1].into(), 3);
        let mul119_out1 = concat51_out1.mul(unsqueeze23_out1.clone());
        let add67_out1 = mul118_out1.add(mul119_out1);
        let mul120_out1 = squeeze35_out1.clone().mul(unsqueeze22_out1);
        let shape66_out1: [i64; 4] = {
            let axes = &squeeze35_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let gather53_out1 = shape66_out1[3] as i64;
        let constant627_out1 = 2i64;
        let div46_out1 = gather53_out1 / constant627_out1;
        let unsqueeze86_out1 = [div46_out1 as i64];
        let slice80_out1 = squeeze35_out1
            .clone()
            .slice(s![.., .., .., 0..unsqueeze86_out1[0]]);
        let unsqueeze87_out1 = [div46_out1 as i64];
        let slice81_out1 =
            squeeze35_out1.slice(s![.., .., .., unsqueeze87_out1[0]..9223372036854775807]);
        let neg24_out1 = slice81_out1.neg();
        let concat52_out1 = burn::tensor::Tensor::cat([neg24_out1, slice80_out1].into(), 3);
        let mul121_out1 = concat52_out1.mul(unsqueeze23_out1);
        let add68_out1 = mul120_out1.add(mul121_out1);
        let shape67_out1: [i64; 4] = {
            let axes = &add67_out1.clone().dims()[0..4];
            let mut output = [0i64; 4];
            for i in 0..4 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let slice82_out1: [i64; 1] = shape67_out1[3..4].try_into().unwrap();
        let cast89_out1 = {
            let shape_array = slice82_out1 as [i64; 1usize];
            let float_array: [f64; 1usize] = shape_array.map(|x| x as f64);
            Tensor::<1>::from_data(
                TensorData::from(float_array),
                (&self.device, burn::tensor::DType::F32),
            )
        };
        let sqrt34_out1 = cast89_out1.sqrt();
        let constant638_out1 = self.constant638.val();
        let div47_out1 = constant638_out1.div(sqrt34_out1);
        let transpose38_out1 = add68_out1.permute([0, 1, 3, 2]);
        let sqrt35_out1 = div47_out1.sqrt();
        let mul122_out1 =
            add67_out1.mul((sqrt35_out1.clone()).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let mul123_out1 =
            transpose38_out1.mul((sqrt35_out1).unsqueeze_dims(&[0isize, 1isize, 2isize]));
        let matmul70_out1 = mul122_out1.matmul(mul123_out1);
        let add69_out1 = matmul70_out1.add(where3_out1);
        let softmax12_out1 = burn::tensor::activation::softmax(add69_out1, 3);
        let matmul71_out1 = softmax12_out1.matmul(squeeze36_out1);
        let transpose39_out1 = matmul71_out1.permute([0, 2, 1, 3]);
        let unsqueeze88_out1 = [gather51_out1 as i64];
        let constant641_out1: [i64; 1] = [384i64];
        let constant640_out1: [i64; 1] = [-1i64];
        let concat53_out1: [i64; 3usize] = [
            &unsqueeze88_out1[..],
            &constant640_out1[..],
            &constant641_out1[..],
        ]
        .concat()
        .try_into()
        .unwrap();
        let reshape27_out1 = transpose39_out1.reshape(concat53_out1);
        let linear46_out1 = self.linear46.forward(reshape27_out1);
        let add70_out1 = add66_out1.add(linear46_out1);
        let layernormalization24_out1 = {
            let dtype = add70_out1.clone().dtype();
            self.layernormalization24
                .forward(add70_out1.clone().cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        let linear47_out1 = self.linear47.forward(layernormalization24_out1);
        let shape68_out1: [i64; 3] = {
            let axes = &linear47_out1.clone().dims()[0..3];
            let mut output = [0i64; 3];
            for i in 0..3 {
                output[i] = axes[i] as i64;
            }
            output
        };
        let constant643_out1: [i64; 1] = [-1i64];
        let gather54_out1: [i64; 1usize] = constant643_out1
            .iter()
            .map(|&idx| {
                let actual_idx = if idx < 0 {
                    (shape68_out1.len() as i64 + idx) as usize
                } else {
                    idx as usize
                };
                shape68_out1[actual_idx]
            })
            .collect::<alloc::vec::Vec<_>>()
            .try_into()
            .unwrap();
        let constant645_out1: [i64; 1] = [1i64];
        let add71_out1 = {
            let __lhs = gather54_out1;
            let __rhs = constant645_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_add(__rhs[__i]))
        };
        let constant646_out1: [i64; 1] = [2i64];
        let div48_out1 = {
            let __lhs = add71_out1;
            let __rhs = constant646_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| {
                if __rhs[__i] != 0 {
                    __lhs[__i] / __rhs[__i]
                } else {
                    __lhs[__i]
                }
            })
        };
        let slice83_out1 = linear47_out1.clone().slice(s![.., .., 0..div48_out1[0]]);
        let constant648_out1: [i64; 1] = [2i64];
        let mul125_out1 = {
            let __lhs = div48_out1;
            let __rhs = constant648_out1;
            core::array::from_fn::<i64, 1usize, _>(|__i| (__lhs[__i]).saturating_mul(__rhs[__i]))
        };
        let slice84_out1 = linear47_out1.slice(s![.., .., div48_out1[0]..mul125_out1[0]]);
        let sigmoid12_out1 = burn::tensor::activation::sigmoid(slice83_out1.clone());
        let mul126_out1 = slice83_out1.mul(sigmoid12_out1);
        let mul127_out1 = mul126_out1.mul(slice84_out1);
        let linear48_out1 = self.linear48.forward(mul127_out1);
        let add72_out1 = add70_out1.add(linear48_out1);
        let layernormalization25_out1 = {
            let dtype = add72_out1.dtype();
            self.layernormalization25
                .forward(add72_out1.cast(burn::tensor::DType::F32))
                .cast(dtype)
        };
        layernormalization25_out1
    }
}

#[derive(Module, Debug)]
pub struct Model {
    submodule1: Submodule1,
    submodule2: Submodule2,
    submodule3: Submodule3,
    submodule4: Submodule4,
    submodule5: Submodule5,
    submodule6: Submodule6,
    submodule7: Submodule7,
    submodule8: Submodule8,
    submodule9: Submodule9,
    submodule10: Submodule10,
    #[module(skip)]
    device: Device,
}

impl Model {
    #[allow(unused_variables)]
    pub fn new(device: &Device) -> Self {
        let submodule1 = Submodule1::new(device);
        let submodule2 = Submodule2::new(device);
        let submodule3 = Submodule3::new(device);
        let submodule4 = Submodule4::new(device);
        let submodule5 = Submodule5::new(device);
        let submodule6 = Submodule6::new(device);
        let submodule7 = Submodule7::new(device);
        let submodule8 = Submodule8::new(device);
        let submodule9 = Submodule9::new(device);
        let submodule10 = Submodule10::new(device);
        Self {
            submodule1,
            submodule2,
            submodule3,
            submodule4,
            submodule5,
            submodule6,
            submodule7,
            submodule8,
            submodule9,
            submodule10,
            device: device.clone(),
        }
    }

    #[allow(clippy::let_and_return, clippy::approx_constant)]
    pub fn forward(&self, input_ids: Tensor<2, Int>, attention_mask: Tensor<2, Int>) -> Tensor<3> {
        let (
            add4_out1,
            gather7_out1,
            cast10_out1,
            where3_out1,
            unsqueeze13_out1,
            unsqueeze14_out1,
            where2_out1,
        ) = self.submodule1.forward(input_ids, attention_mask);
        let (add10_out1, unsqueeze22_out1, unsqueeze23_out1) =
            self.submodule2
                .forward(add4_out1, gather7_out1, cast10_out1, where3_out1.clone());
        let add16_out1 = self.submodule3.forward(
            add10_out1,
            unsqueeze22_out1.clone(),
            unsqueeze23_out1.clone(),
            where3_out1.clone(),
        );
        let add22_out1 = self.submodule4.forward(
            add16_out1,
            unsqueeze13_out1.clone(),
            unsqueeze14_out1.clone(),
            where2_out1.clone(),
        );
        let add28_out1 = self.submodule5.forward(
            add22_out1,
            unsqueeze22_out1.clone(),
            unsqueeze23_out1.clone(),
            where3_out1.clone(),
        );
        let add34_out1 = self.submodule6.forward(
            add28_out1,
            unsqueeze22_out1.clone(),
            unsqueeze23_out1.clone(),
            where3_out1.clone(),
        );
        let add40_out1 = self.submodule7.forward(
            add34_out1,
            unsqueeze13_out1.clone(),
            unsqueeze14_out1.clone(),
            where2_out1.clone(),
        );
        let add58_out1 = self.submodule8.forward(
            add40_out1,
            unsqueeze22_out1.clone(),
            unsqueeze23_out1.clone(),
            where3_out1.clone(),
            unsqueeze13_out1,
            unsqueeze14_out1,
            where2_out1,
        );
        let add64_out1 = self.submodule9.forward(
            add58_out1,
            unsqueeze22_out1.clone(),
            unsqueeze23_out1.clone(),
            where3_out1.clone(),
        );
        let layernormalization25_out1 =
            self.submodule10
                .forward(add64_out1, unsqueeze22_out1, unsqueeze23_out1, where3_out1);
        layernormalization25_out1
    }
}
