#include "pairwise.h"

typedef Eigen::Matrix<float, Eigen::Dynamic, Eigen::Dynamic, Eigen::RowMajor> NumpyMatF;

// Same arithmetic as pydensecrf's expAndNormalize, but written on the output column so no
// per-pixel VectorXf is heap-allocated (that allocation dominated the inference loop).
static void expAndNormalize(MatrixXf& out, const MatrixXf& in) {
    out.resize(in.rows(), in.cols());
    for (int i = 0; i < out.cols(); i++) {
        auto b = out.col(i);
        b = (in.col(i).array() - in.col(i).maxCoeff()).exp();
        b /= b.sum();
    }
}

// Mirrors DenseCRF2D::addPairwiseGaussian(1, 1) + addPairwiseBilateral(23, 23, 7, 7, 7) followed
// by DenseCRF::inference(num_iterations).
extern "C" void run_densecrf(
    const float* unary,
    int width,
    int height,
    int n_classes,
    const unsigned char* image,
    int num_iterations,
    float* out_probs)
{
    const int n = width * height;
    const MatrixXf unary_mat = Eigen::Map<const NumpyMatF>(unary, n_classes, n);

    const float sxy = 1, bxy = 23, srgb = 7;
    MatrixXf gaussian(2, n), bilateral(5, n);
    for (int j = 0; j < height; j++)
        for (int i = 0; i < width; i++) {
            const int p = j * width + i;
            gaussian(0, p) = i / sxy;
            gaussian(1, p) = j / sxy;
            bilateral(0, p) = i / bxy;
            bilateral(1, p) = j / bxy;
            bilateral(2, p) = image[p * 3 + 0] / srgb;
            bilateral(3, p) = image[p * 3 + 1] / srgb;
            bilateral(4, p) = image[p * 3 + 2] / srgb;
        }
    PairwisePotential gaussian_term(gaussian, new PottsCompatibility(3), DIAG_KERNEL, NO_NORMALIZATION);
    PairwisePotential bilateral_term(bilateral, new PottsCompatibility(20), DIAG_KERNEL, NO_NORMALIZATION);

    MatrixXf q, tmp1, tmp2;
    expAndNormalize(q, -unary_mat);
    for (int it = 0; it < num_iterations; it++) {
        tmp1 = -unary_mat;
        gaussian_term.apply(tmp2, q);
        tmp1 -= tmp2;
        bilateral_term.apply(tmp2, q);
        tmp1 -= tmp2;
        expAndNormalize(q, tmp1);
    }
    Eigen::Map<NumpyMatF>(out_probs, n_classes, n) = q;
}
